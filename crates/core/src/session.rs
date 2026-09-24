//! Port of the session layer: `EditorSession` and its extensions
//! (`EditorSession.swift`, `Selection.swift`, `LayerAppearance.swift`,
//! `LayerMask.swift`, `LayerGroups.swift`, `LayerMerge.swift`,
//! `LayerFlip.swift`, `DocumentHistory` integration, `Crop.swift`,
//! `CanvasSize`/`ImageSize`/`Trim` operations).

#![allow(dead_code)]

use crate::blend::LayerBlendMode;
use crate::canvas::{
    mirrored_transform, resize_canvas, resize_image, trim as trim_document, CanvasSizeOptions,
    CropDrag, CropDragMode, CropGeometry, TrimOptions,
};
use crate::document::{
    CanvasDocument, CanvasGuide, GuideAxis, ImageLayer, LayerMask, LayerTextStyle, NavigationTool,
    PaletteColor,
};
use crate::geometry::{CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize};
use crate::history::DocumentHistory;
use crate::raster::{Bitmap, ImportedImage};
use crate::selection::{
    drag_box_rect, DocumentSelection, LassoDraft, LassoKind, SelectionMode, SelectionPath,
};
use crate::tools::brush::RasterEdit;
use crate::transform::{LayerSampling, LayerTransform, TransformEdit};
use uuid::Uuid;

/// The Blur tool's modes (`BlurToolMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlurToolMode {
    #[default]
    Liquify,
    Blur,
    Smudge,
}

/// The Brush's two modes (B and E).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BrushToolMode {
    #[default]
    Paint,
    Erase,
}

/// Spot Healing's modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpotHealingMode {
    #[default]
    ContentAware,
    CreateTexture,
    ProximityMatch,
}

/// The Magic tool's settings (`WandSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WandSettings {
    /// How far (0–255) each channel may differ from the sampled color.
    pub tolerance: i32,
    /// Pixels either side of the click averaged into the color to match.
    pub sample_radius: usize,
    /// Only similar pixels connected to the clicked one.
    pub contiguous: bool,
    /// Read the visible composite rather than just the active layer.
    pub sample_all_layers: bool,
}

impl Default for WandSettings {
    fn default() -> Self {
        WandSettings {
            tolerance: 32,
            sample_radius: 0,
            contiguous: true,
            sample_all_layers: false,
        }
    }
}

/// Clone Stamp settings (`CloneSettings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloneSettings {
    /// Aligned strokes keep the offset from brush to source.
    pub aligned: bool,
    /// Copy from every visible layer as shown.
    pub sample_all_layers: bool,
}

impl Default for CloneSettings {
    fn default() -> Self {
        CloneSettings {
            aligned: true,
            sample_all_layers: false,
        }
    }
}

/// Gradient tool settings (`GradientSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientSettings {
    pub shape: crate::tools::brush::GradientShape,
    pub foreground_to_background: bool,
    pub reversed: bool,
    pub opacity: f64,
}

impl Default for GradientSettings {
    fn default() -> Self {
        GradientSettings {
            shape: crate::tools::brush::GradientShape::Linear,
            foreground_to_background: false,
            reversed: false,
            opacity: 1.0,
        }
    }
}

/// `Select > Modify` operations that ask for an amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionAmountOperation {
    Expand,
    Contract,
    Feather,
}

/// The session: the open document plus every tool state. The Swift class is
/// Text being typed (`TextDraft`): which layer it edits, where it sits, and
/// the style it will carry.
#[derive(Clone)]
pub struct TextDraft {
    pub document_id: Uuid,
    pub layer_id: Option<Uuid>,
    pub origin: CGPoint,
    pub transform: Option<LayerTransform>,
    pub style: crate::document::LayerTextStyle,
}

/// Pixels copied from the canvas, with where they came from so Paste can put
/// them back in place (`PixelClipboard`).
#[derive(Clone)]
pub struct PixelClipboard {
    pub image: Bitmap,
    pub origin: CGPoint,
}

/// `@Observable`; here the UI reads the fields directly and mutation methods
/// keep the same guards and undo names.
pub struct EditorSession {
    pub document: Option<CanvasDocument>,
    pub tool: NavigationTool,
    pub active_layer_id: Option<Uuid>,
    pub selected_layer_ids: Vec<Uuid>,
    pub is_mask_selected: bool,
    pub collapsed_group_ids: std::collections::HashSet<Uuid>,
    pub history: DocumentHistory,

    // Appearance preview
    pub blend_preview: Option<(Uuid, LayerBlendMode)>,
    pub opacity_edit_layer_id: Option<Uuid>,

    // Painting
    pub brush_settings: crate::tools::brush::BrushSettings,
    pub brush_mode: BrushToolMode,
    pub spot_healing_mode: SpotHealingMode,
    pub blur_mode: BlurToolMode,
    pub wand_mode: crate::selection::WandMode,
    pub wand_settings: WandSettings,
    pub clone_settings: CloneSettings,
    pub clone_source: Option<CGPoint>,
    pub clone_offset: Option<CGSize>,
    pub mask_paint_white: bool,
    pub background_color: PaletteColor,
    pub gradient_settings: GradientSettings,

    // Selection editing
    pub lasso_draft: Option<LassoDraft>,
    pub lasso_kind: LassoKind,
    pub marquee_kind: LassoKind,
    pub selection_mode_choice: SelectionMode,
    pub selection_antialiased: bool,
    pub selection_feather_amount: i64,
    pub selection_expand_amount: i64,
    pub selection_contract_amount: i64,
    pub selection_amount_operation: Option<SelectionAmountOperation>,
    pub selection_move_origin: Option<DocumentSelection>,

    // Clipboard (`pixelClipboard`)
    pub pixel_clipboard: Option<PixelClipboard>,

    // Text being typed (`textDraft`)
    pub text_draft: Option<TextDraft>,
    pub canvas_focus_request: u64,

    // Transform
    pub transform_edit: Option<TransformEdit>,
    pub locks_transform_ratio: bool,
    pub snapping_enabled: bool,
    /// The guide lines the last snap landed on, for the canvas to draw.
    pub snap_guides: (Vec<CGFloat>, Vec<CGFloat>),

    // Crop
    pub crop_rect: Option<CGRect>,
    pub crop_ratio_choice: String,
    pub crop_drag: Option<CropDrag>,

    // Sheets (the UI reads these to present its dialogs)
    pub shows_new_document: bool,
    pub levels: Option<crate::adjust::LevelsSettings>,
    pub levels_layer_id: Option<Uuid>,
    pub hue_saturation: Option<crate::adjust::HueSaturationSettings>,
    pub hue_saturation_layer_id: Option<Uuid>,

    // Guides
    pub shows_guides: bool,
    pub shows_rulers: bool,
    pub shows_grid: bool,
    pub shows_pixel_grid: bool,
    pub snap_to_guides: bool,
    pub snap_to_grid: bool,
    pub snap_to_layers: bool,
    pub snap_to_document_bounds: bool,
    pub locks_guides: bool,

    // Text
    pub text_defaults: LayerTextStyle,

    // Shape tool
    pub shape_kind: crate::document::ShapeKind,
    /// Corner radius in pixels for rectangles the Shape tool draws.
    pub shape_corner_radius: f64,
    /// A Line shape's thickness in document pixels.
    pub shape_line_width: f64,

    // Errors (the UI surfaces these)
    pub brush_error: Option<String>,
}

impl Default for EditorSession {
    fn default() -> Self {
        EditorSession {
            document: None,
            tool: NavigationTool::Move,
            active_layer_id: None,
            selected_layer_ids: Vec::new(),
            is_mask_selected: false,
            collapsed_group_ids: std::collections::HashSet::new(),
            pixel_clipboard: None,
            text_draft: None,
            canvas_focus_request: 0,
            history: DocumentHistory::default(),
            blend_preview: None,
            opacity_edit_layer_id: None,
            brush_settings: crate::tools::brush::BrushSettings::default(),
            brush_mode: BrushToolMode::Paint,
            spot_healing_mode: SpotHealingMode::ContentAware,
            blur_mode: BlurToolMode::Liquify,
            wand_mode: crate::selection::WandMode::Wand,
            wand_settings: WandSettings::default(),
            clone_settings: CloneSettings::default(),
            clone_source: None,
            clone_offset: None,
            mask_paint_white: false,
            background_color: PaletteColor::WHITE,
            gradient_settings: GradientSettings::default(),
            lasso_draft: None,
            lasso_kind: LassoKind::Freehand,
            marquee_kind: LassoKind::Rectangle,
            selection_mode_choice: SelectionMode::Replace,
            selection_antialiased: true,
            selection_feather_amount: 2,
            selection_expand_amount: 1,
            selection_contract_amount: 1,
            selection_amount_operation: None,
            selection_move_origin: None,
            transform_edit: None,
            locks_transform_ratio: true,
            snapping_enabled: true,
            snap_guides: (Vec::new(), Vec::new()),
            crop_rect: None,
            crop_ratio_choice: "Free".to_string(),
            crop_drag: None,
            shows_new_document: false,
            levels: None,
            levels_layer_id: None,
            hue_saturation: None,
            hue_saturation_layer_id: None,
            shows_guides: true,
            shows_rulers: false,
            shows_grid: false,
            shows_pixel_grid: true,
            snap_to_guides: true,
            snap_to_grid: false,
            snap_to_layers: true,
            snap_to_document_bounds: true,
            locks_guides: false,
            shape_kind: crate::document::ShapeKind::Rectangle,
            shape_corner_radius: 0.0,
            shape_line_width: 4.0,
            text_defaults: LayerTextStyle::default(),
            brush_error: None,
        }
    }
}

impl EditorSession {
    pub fn new() -> Self {
        Self::default()
    }

    // MARK: Document

    /// `createDocument` — File > New.
    pub fn create_document(&mut self, width: usize, height: usize, empty_layer: bool) {
        if !(1..=30_000).contains(&width) || !(1..=30_000).contains(&height) {
            return;
        }
        self.commit_transform();
        self.begin_edit("New Canvas");
        let mut document = CanvasDocument::new(width, height);
        let layer = if empty_layer {
            Some(ImageLayer::blank("Layer 1", document.size()))
        } else {
            None
        };
        let layer_id = layer.as_ref().map(|l| l.id);
        if let Some(layer) = layer {
            document.layers = vec![layer];
        }
        self.document = Some(document);
        self.active_layer_id = layer_id;
        self.selected_layer_ids = layer_id.map(|i| vec![i]).unwrap_or_default();
        self.history.end(&self.document, self.active_layer_id);
        self.shows_new_document = false;
    }

    pub fn document_size(&self) -> Option<CGSize> {
        self.document.as_ref().map(|d| d.size())
    }

    pub fn active_layer(&self) -> Option<&ImageLayer> {
        let id = self.active_layer_id?;
        self.document.as_ref()?.layer(id)
    }

    pub fn selection(&self) -> Option<&DocumentSelection> {
        self.document.as_ref()?.selection.as_ref()
    }

    // MARK: Undo

    fn begin_edit(&mut self, name: &str) {
        self.history.begin(name, &self.document, self.active_layer_id);
    }

    fn end_edit(&mut self) {
        self.history.end(&self.document, self.active_layer_id);
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo(&mut self) {
        if let Some(snapshot) = self.history.undo() {
            self.document = snapshot.document;
            self.active_layer_id = snapshot.active_layer_id;
            if let Some(id) = self.active_layer_id {
                self.selected_layer_ids = vec![id];
            }
        }
    }

    pub fn redo(&mut self) {
        if let Some(snapshot) = self.history.redo() {
            self.document = snapshot.document;
            self.active_layer_id = snapshot.active_layer_id;
            if let Some(id) = self.active_layer_id {
                self.selected_layer_ids = vec![id];
            }
        }
    }

    pub fn is_modified(&self) -> bool {
        self.history.is_modified()
    }

    pub fn mark_saved(&mut self) {
        self.history.mark_saved();
    }

    // MARK: Layers

    pub fn select_layer(&mut self, id: Option<Uuid>) {
        // Switching layers commits a pending transform first (selectLayer).
        if id != self.active_layer_id {
            self.commit_transform();
            self.is_mask_selected = false;
        }
        self.active_layer_id = id;
        self.selected_layer_ids = id.map(|i| vec![i]).unwrap_or_default();
    }

    /// `addBlankLayer`.
    pub fn add_blank_layer(&mut self) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let names: Vec<String> = document.layers.iter().map(|l| l.name.clone()).collect();
        let mut number = 1;
        while names.contains(&format!("Layer {number}")) {
            number += 1;
        }
        let mut layer = ImageLayer::blank(format!("Layer {number}"), document.size());
        layer.parent_id = match self.active_layer() {
            Some(active) if active.is_group => self.active_layer_id,
            Some(active) => active.parent_id,
            None => None,
        };
        let insertion = self
            .active_layer_id
            .and_then(|id| document.layer_index(id))
            .map(|i| i + 1)
            .unwrap_or(document.layers.len());
        self.begin_edit("New Blank Layer");
        let id = layer.id;
        if let Some(doc) = self.document.as_mut() {
            doc.layers.insert(insertion.min(doc.layers.len()), layer);
            self.active_layer_id = Some(id);
            self.selected_layer_ids = vec![id];
        }
        self.end_edit();
    }

    /// `deleteLayer` — a selected folder takes its contents.
    pub fn delete_layer(&mut self, id: Uuid) {
        self.delete_layer_with_baked(id, Default::default());
    }

    fn delete_layer_with_baked(
        &mut self,
        id: Uuid,
        baked: std::collections::HashMap<Uuid, ImportedImage>,
    ) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(index) = document.layer_index(id) else {
            return;
        };
        let removed = document.descendant_ids(id);
        self.begin_edit("Delete Layer");
        if let Some(doc) = self.document.as_mut() {
            doc.layers.retain(|l| !removed.contains(&l.id));
            // Layers clipped to a deleted layer keep their pixels — baked in
            // when the delete chose to bake — unlinked.
            for layer in doc.layers.iter_mut() {
                if layer
                    .mask_source_id
                    .map(|source| removed.contains(&source))
                    .unwrap_or(false)
                {
                    layer.mask_source_id = None;
                    if let Some(image) = baked.get(&layer.id) {
                        layer.asset = Some(image.clone());
                    }
                }
            }
        }
        // The deleted active layer's selection moves to a neighbor
        // (finishDeletingLayer): layers[min(index, count-1)].
        if self.active_layer_id.map(|a| removed.contains(&a)).unwrap_or(false) {
            let count = self.document.as_ref().map(|d| d.layers.len()).unwrap_or(0);
            if count == 0 {
                self.active_layer_id = None;
                self.selected_layer_ids.clear();
            } else {
                let neighbor = self.document.as_ref().map(|d| d.layers[index.min(count - 1)].id);
                self.active_layer_id = neighbor;
                self.selected_layer_ids = neighbor.map(|n| vec![n]).unwrap_or_default();
            }
        }
        self.end_edit();
    }

    pub fn delete_active_layer(&mut self) {
        if let Some(id) = self.active_layer_id {
            self.delete_layer(id);
        }
    }

    pub fn rename_layer(&mut self, id: Uuid, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        if self.document.as_ref().and_then(|d| d.layer(id)).is_some() {
            self.begin_edit("Rename Layer");
            if let Some(doc) = self.document.as_mut() {
                doc.layer_mut(id).unwrap().name = name.to_string();
            }
            self.end_edit();
        }
    }

    pub fn toggle_layer_visibility(&mut self, id: Uuid) {
        let was_visible = self
            .document
            .as_ref()
            .and_then(|d| d.layer(id))
            .map(|l| l.is_visible);
        if let Some(visible) = was_visible {
            self.begin_edit(if visible { "Hide Layer" } else { "Show Layer" });
            if let Some(doc) = self.document.as_mut() {
                doc.layer_mut(id).unwrap().is_visible = !visible;
            }
            self.end_edit();
        }
    }

    /// `moveActiveLayer(by:)` — within its siblings.
    pub fn move_active_layer(&mut self, offset: i64) {
        let Some(active_id) = self.active_layer_id else {
            return;
        };
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(active) = document.layer(active_id) else {
            return;
        };
        let siblings: Vec<Uuid> = document
            .layers
            .iter()
            .filter(|l| l.parent_id == active.parent_id)
            .map(|l| l.id)
            .collect();
        let Some(index) = siblings.iter().position(|i| *i == active_id) else {
            return;
        };
        let target = index as i64 + offset;
        if target < 0 || target >= siblings.len() as i64 {
            return;
        }
        let other = siblings[target as usize];
        let a = document.layer_index(active_id).unwrap();
        let b = document.layer_index(other).unwrap();
        self.begin_edit("Reorder Layers");
        if let Some(doc) = self.document.as_mut() {
            doc.layers.swap(a, b);
        }
        self.end_edit();
    }

    /// `duplicateActiveLayer` — a folder copies with all its descendants,
    /// parent and mask-source links remapped to the copies.
    pub fn duplicate_active_layer(&mut self) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(source_id) = self.active_layer_id else {
            return;
        };
        let Some(source) = document.layer(source_id).cloned() else {
            return;
        };
        let Some(index) = document.layer_index(source_id) else {
            return;
        };
        let included = document.descendant_ids(source_id);
        let originals: Vec<ImageLayer> = document
            .layers
            .iter()
            .filter(|l| included.contains(&l.id))
            .cloned()
            .collect();
        if document.layers.len() + originals.len() > 10_000 {
            return;
        }
        let mapping: std::collections::HashMap<Uuid, Uuid> = originals
            .iter()
            .map(|l| (l.id, Uuid::new_v4()))
            .collect();
        let mut copies = originals;
        for copy in copies.iter_mut() {
            copy.id = mapping[&copy.id];
            if copy.id == mapping[&source_id] {
                copy.name = format!("{} copy", copy.name);
            }
            if let Some(parent) = copy.parent_id {
                copy.parent_id = Some(mapping.get(&parent).copied().unwrap_or(parent));
            }
            if let Some(source) = copy.mask_source_id {
                copy.mask_source_id = Some(mapping.get(&source).copied().unwrap_or(source));
            }
        }
        self.begin_edit("Duplicate Layer");
        if let Some(doc) = self.document.as_mut() {
            let position = (index + 1).min(doc.layers.len());
            for (offset, copy) in copies.into_iter().enumerate() {
                doc.layers.insert(position + offset, copy);
            }
            let copy_id = mapping[&source_id];
            self.active_layer_id = Some(copy_id);
            self.selected_layer_ids = vec![copy_id];
        }
        self.end_edit();
    }

    /// `finishShape`: fills the dragged shape with the foreground color on a
    /// new layer above the active one, in one undo step named after the kind.
    pub fn add_shape_layer(
        &mut self,
        shape: crate::document::LayerShape,
        rect: CGRect,
        name: String,
    ) -> Option<Uuid> {
        let document = self.document.clone()?;
        if rect.width() < 1.0 || rect.height() < 1.0 {
            return None;
        }
        if (rect.width() as usize).saturating_mul(rect.height() as usize)
            > crate::document::LayerShape::MAX_SHAPE_PIXELS
        {
            self.brush_error =
                Some("That shape is too large. A shape can cover up to 100 megapixels.".into());
            return None;
        }
        let transform = LayerTransform::new(rect.origin, rect.size);
        let undo_name = shape.kind.label().to_string();
        let raster = crate::render::render_shape_image(&shape, transform.size.width.round() as usize, transform.size.height.round() as usize);
        let mut layer = ImageLayer::with_asset(ImportedImage::new(raster, name.clone()), rect.origin);
        layer.transform = transform;
        layer.shape = Some(shape);
        // Above the active layer, in its folder.
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
        // The undo step is named after the kind; the layer gets the numbered name.
        self.begin_edit(&undo_name);
        let id = layer.id;
        if let Some(doc) = self.document.as_mut() {
            doc.layers.insert(index.min(doc.layers.len()), layer);
            self.active_layer_id = Some(id);
            self.selected_layer_ids = vec![id];
        }
        self.end_edit();
        Some(id)
    }

    /// `"Rectangle 1"`, `"Ellipse 2"`, ... skipping names already in the document.
    pub fn next_shape_name(&self, kind: crate::document::ShapeKind) -> String {
        let names: std::collections::HashSet<String> = self
            .document
            .as_ref()
            .map(|d| d.layers.iter().map(|l| l.name.clone()).collect())
            .unwrap_or_default();
        let mut number = 1;
        while names.contains(&format!("{} {number}", kind.label())) {
            number += 1;
        }
        format!("{} {number}", kind.label())
    }

    // MARK: Appearance

    /// `canEditLayers` — the states the core session models; the app
    /// conjuncts its own stroke/sheet state on top.
    pub fn can_edit_layers(&self) -> bool {
        self.document.is_some() && self.transform_edit.is_none() && self.crop_rect.is_none()
    }

    pub fn can_edit_appearance(&self) -> bool {
        self.can_edit_layers()
            && self.selected_layer_ids.len() == 1
            && self.active_layer().map(|l| !l.is_group).unwrap_or(false)
    }

    pub fn displayed_blend_mode(&self, layer: &ImageLayer) -> LayerBlendMode {
        if let Some((id, mode)) = self.blend_preview {
            if id == layer.id && self.active_layer_id == Some(layer.id) {
                return mode;
            }
        }
        layer.blend_mode
    }

    /// Shift-+ / Shift-− steps the blend mode through the menu order.
    pub fn cycle_blend_mode(&mut self, forward: bool) {
        if !self.can_edit_appearance() {
            return;
        }
        let Some(layer) = self.active_layer() else {
            return;
        };
        let modes = LayerBlendMode::ALL;
        let index = modes.iter().position(|m| *m == layer.blend_mode).unwrap_or(0);
        let next = modes[(index + if forward { 1 } else { modes.len() - 1 }) % modes.len()];
        self.set_layer_blend_mode(next);
    }

    pub fn set_layer_blend_mode(&mut self, mode: LayerBlendMode) {
        self.blend_preview = None;
        if !self.can_edit_appearance() {
            return;
        }
        let Some(id) = self.active_layer_id else {
            return;
        };
        self.finish_opacity_edit();
        self.begin_edit("Layer Blend Mode");
        if let Some(doc) = self.document.as_mut() {
            doc.layer_mut(id).unwrap().blend_mode = mode;
        }
        self.end_edit();
    }

    pub fn begin_opacity_edit(&mut self) {
        if self.opacity_edit_layer_id.is_none() {
            if let Some(id) = self.active_layer_id {
                self.begin_edit("Layer Opacity");
                self.opacity_edit_layer_id = Some(id);
            }
        }
    }

    pub fn finish_opacity_edit(&mut self) {
        if self.opacity_edit_layer_id.is_some() {
            self.opacity_edit_layer_id = None;
            self.end_edit();
        }
    }

    pub fn set_layer_opacity(&mut self, opacity: f64) {
        if !opacity.is_finite() {
            return;
        }
        let Some(id) = self.opacity_edit_layer_id.or(self.active_layer_id) else {
            return;
        };
        let standalone = self.opacity_edit_layer_id.is_none();
        if standalone {
            self.begin_edit("Layer Opacity");
        }
        if let Some(doc) = self.document.as_mut() {
            if let Some(layer) = doc.layer_mut(id) {
                layer.opacity = opacity.clamp(0.0, 1.0);
            }
        }
        if standalone {
            self.end_edit();
        }
    }

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
        let Some(draft) = self.lasso_draft.as_mut() else {
            return;
        };
        if let Some(last) = draft.points.last() {
            if last.distance_to(to) < 0.25 {
                return;
            }
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
        self.begin_edit("Move Selection");
        if let Some(doc) = self.document.as_mut() {
            if let Some(selection) = doc.selection.as_mut() {
                selection.path = selection.path.translated(dx, dy);
            }
        }
        self.end_edit();
    }

    pub fn can_modify_selection(&self) -> bool {
        self.selection().map(|s| !s.is_empty()).unwrap_or(false)
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
                self.brush_error = Some("The Magic Wand can't trace this selection — try a lower tolerance.".into());
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
            if let Some(layer) = self.active_layer() {
                if !layer.is_group {
                    if let Some(asset) = layer.asset.as_ref() {
                        crate::render::draw_layer_pixels(asset.image.as_ref(), &layer.transform, &mut out);
                    }
                }
            }
            out
        }
    }

    // MARK: Masks

    /// `canEditMask`: canEditLayers && one selected layer.
    pub fn can_edit_mask(&self) -> bool {
        self.can_edit_layers() && self.selected_layer_ids.len() == 1 && self.active_layer().is_some()
    }

    pub fn add_layer_mask(&mut self, revealing: bool) {
        if !self.can_edit_mask() || self.active_layer().map(|l| l.mask.is_some()).unwrap_or(true) {
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
            self.begin_edit(if enabled { "Disable Layer Mask" } else { "Enable Layer Mask" });
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
        if self.active_layer().map(|l| l.mask.is_some()).unwrap_or(false) {
            self.finish_opacity_edit();
            self.begin_edit("Delete Layer Mask");
            if let Some(doc) = self.document.as_mut() {
                doc.layer_mut(id).unwrap().mask = None;
            }
            self.is_mask_selected = false;
            self.end_edit();
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
        let layer = self.active_layer()?;
        if layer.is_group {
            return None;
        }
        let (source, transform) = if self.is_mask_selected {
            (
                layer.mask.as_ref().map(|m| m.asset.image.as_ref().clone()),
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
            self.is_mask_selected,
            self.brush_settings,
            CGSize::new(document.width as f64, document.height as f64),
            clip,
        ))
    }

    /// `commitRasterEdit` — the composed grid becomes the targeted layer's (or
    /// mask's) pixels as one undo step.
    fn commit_pixel_edit(&mut self, edit: &RasterEdit, composed: &Bitmap, name: &str) {
        let (mut image, transform, _) = edit.snapshot_of(composed);
        let Some(id) = self.active_layer_id else {
            return;
        };
        self.begin_edit(name);
        if let Some(document) = self.document.as_mut() {
            if let Some(layer) = document.layer_mut(id) {
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
        self.commit_pixel_edit(&edit, &composed, if edit.is_mask { "Fill Mask" } else { "Fill" });
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
        let grid_transform = if mask { layer.mask_transform() } else { layer.transform };
        let to_doc = crate::raster::pixel_to_document(&grid_transform, image.width as f64, image.height as f64);
        for y in 0..image.height {
            for x in 0..image.width {
                let coverage = match &clip {
                    Some(clip) => {
                        let doc = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5).applying(&to_doc);
                        if doc.x < 0.0 || doc.y < 0.0 || doc.x >= document.width as f64 || doc.y >= document.height as f64 {
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
                    image.data[o + c] =
                        (inverted as f32 * coverage + image.data[o + c] as f32 * (1.0 - coverage)).round() as u8;
                }
            }
        }
        self.begin_edit(if mask { "Invert Mask" } else { "Invert" });
        if let Some(document) = self.document.as_mut() {
            if let Some(current) = document.layer_mut(layer.id) {
                if mask {
                    if let Some(m) = current.mask.as_ref() {
                        current.mask = Some(m.replacing(ImportedImage::new(image, layer.name.clone())));
                    }
                } else {
                    current.asset = Some(ImportedImage::new(image, layer.name.clone()));
                }
            }
        }
        self.end_edit();
    }

    /// `addMask(revealing:)` — with a selection, the mask shows (or hides)
    /// just the selected pixels of the layer's own grid; without one it is the
    /// plain reveal/hide mask.
    pub fn add_mask_from_selection(&mut self, revealing: bool) {
        if self.selection().is_none() {
            self.add_layer_mask(revealing);
            return;
        }
        if !self.can_edit_mask() || self.active_layer().map(|l| l.mask.is_some()).unwrap_or(true) {
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
            let to_doc = crate::raster::pixel_to_document(&layer.transform, width as f64, height as f64);
            for y in 0..height {
                for x in 0..width {
                    let doc = CGPoint::new(x as f64 + 0.5, y as f64 + 0.5).applying(&to_doc);
                    if doc.x < 0.0 || doc.y < 0.0 || doc.x >= document.width as f64 || doc.y >= document.height as f64 {
                        continue;
                    }
                    let a = clip.alpha_at(doc.x.floor() as usize, doc.y.floor() as usize);
                    if a <= 0.0 {
                        continue;
                    }
                    let o = (y * width + x) * 4;
                    let v = grid.data[o] as f32;
                    let value = (v * (1.0 - a) + fill as f32 * a) as u8;
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
            settings.shadows = crate::adjust::AdjustmentColor::from_palette(self.palette_color(false));
            settings.highlights = crate::adjust::AdjustmentColor::from_palette(self.palette_color(true));
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
            document.layers.insert(index.min(document.layers.len()), layer);
            self.active_layer_id = Some(id);
        }
        self.end_edit();
    }

    /// `updateAdjustment(id:value:)`.
    pub fn update_adjustment(&mut self, id: Uuid, value: crate::adjustment::LayerAdjustment) {
        if !value.is_valid() {
            return;
        }
        if let Some(document) = self.document.as_mut() {
            if let Some(layer) = document.layer_mut(id) {
                layer.adjustment = Some(value);
            }
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
            if clip.coverage.is_none() {
                return None;
            }
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
            let fill = (background * 255.0) as u8;
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
                            out.data[o + c] = (out.data[o + c] as f32 * a) as u8;
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
            if clip.coverage.is_none() {
                return None;
            }
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
            document.layers.insert(index.min(document.layers.len()), layer);
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

    // MARK: Multi-select and bulk layer ops (LayerGroups.swift)

    /// `selectLayers` — the selection is intersected with the document's
    /// layers; switching it commits a pending transform first.
    pub fn select_layers(&mut self, ids: &[Uuid], primary: Option<Uuid>) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let known: std::collections::HashSet<Uuid> =
            document.layers.iter().map(|l| l.id).collect();
        let mut valid: Vec<Uuid> = Vec::new();
        for id in ids {
            if known.contains(id) && !valid.contains(id) {
                valid.push(*id);
            }
        }
        let same = valid.len() == self.selected_layer_ids.len()
            && valid.iter().all(|id| self.selected_layer_ids.contains(id));
        if !same {
            self.commit_transform();
        }
        self.active_layer_id = match primary {
            Some(primary) if valid.contains(&primary) => Some(primary),
            _ => valid.first().copied(),
        };
        self.selected_layer_ids = valid;
    }

    /// Cmd-Shift-click: adds a layer to the selection, or takes it out again
    /// when it is already in it (`extendSelection`).
    pub fn extend_selection(&mut self, id: Uuid) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        if !document.layers.iter().any(|l| l.id == id) {
            return;
        }
        let mut ids = self.selected_layer_ids.clone();
        if ids.contains(&id) && ids.len() > 1 {
            ids.retain(|i| *i != id);
            let primary = if self.active_layer_id == Some(id) { ids.first().copied() } else { self.active_layer_id };
            self.select_layers(&ids, primary);
        } else {
            ids.push(id);
            self.select_layers(&ids, Some(id));
        }
    }

    /// `groupSelectedLayers` — wraps the selection in a new "Folder N" at
    /// their common parent. A selected folder carries its subtree, and
    /// selected descendants are not pulled out of it.
    pub fn group_selected_layers(&mut self) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        if document.layers.len() >= 10_000 {
            return;
        }
        let by_id: std::collections::HashMap<Uuid, &ImageLayer> =
            document.layers.iter().map(|l| (l.id, l)).collect();
        let selected: std::collections::HashSet<Uuid> =
            self.selected_layer_ids.iter().copied().collect();
        if selected.is_empty() {
            return;
        }
        let ancestors = |id: Uuid, by_id: &std::collections::HashMap<Uuid, &ImageLayer>| -> Vec<Option<Uuid>> {
            let mut result = Vec::new();
            let mut parent = by_id.get(&id).and_then(|l| l.parent_id);
            while let Some(current) = parent {
                result.push(Some(current));
                parent = by_id.get(&current).and_then(|l| l.parent_id);
            }
            result.push(None);
            result
        };
        let in_selected =
            |chain: &Vec<Option<Uuid>>| chain.iter().any(|a| a.map(|a| selected.contains(&a)).unwrap_or(false));
        // The topmost selected branches: roots of the selected forest.
        let root_ids: Vec<Uuid> = selected
            .iter()
            .copied()
            .filter(|id| !in_selected(&ancestors(*id, &by_id)))
            .collect();
        let root_set: std::collections::HashSet<Uuid> = root_ids.iter().copied().collect();
        // Hierarchy order keeps the relative stacking of what moves.
        let ordered: Vec<Uuid> = document
            .hierarchy_entries(false, &std::collections::HashSet::new())
            .into_iter()
            .map(|e| e.layer.id)
            .filter(|id| root_set.contains(id))
            .collect();
        if ordered.is_empty() {
            return;
        }
        // The nearest ancestor chain every selected branch shares.
        let parent: Option<Uuid> = ordered.first().and_then(|first| {
            let first_chain = ancestors(*first, &by_id);
            first_chain
                .iter()
                .copied()
                .find(|candidate| {
                    ordered.iter().all(|id| {
                        ancestors(*id, &by_id)
                            .iter()
                            .any(|a| *a == *candidate)
                    })
                })
        })
        .flatten();
        let mut names: std::collections::HashSet<String> =
            document.layers.iter().map(|l| l.name.clone()).collect();
        let mut number = 1;
        while names.contains(&format!("Folder {number}")) {
            number += 1;
        }
        let mut group = ImageLayer::blank(format!("Folder {number}"), document.size());
        group.is_group = true;
        group.parent_id = parent;
        // The wrapper sits where the topmost selected branch sits.
        let branches: Vec<Uuid> = ordered
            .iter()
            .map(|id| {
                let mut branch = *id;
                while let Some(next) = by_id.get(&branch).and_then(|l| l.parent_id) {
                    if Some(next) == parent {
                        break;
                    }
                    branch = next;
                }
                branch
            })
            .collect();
        let highest = document.layers.iter().rposition(|l| branches.contains(&l.id));
        let insertion = highest
            .map(|h| document.layers[..=h].iter().filter(|l| !root_set.contains(&l.id)).count())
            .unwrap_or(document.layers.len());
        let mut layers = document
            .layers
            .iter()
            .filter(|l| !root_set.contains(&l.id))
            .cloned()
            .collect::<Vec<ImageLayer>>();
        let group_id = group.id;
        layers.insert(insertion.min(layers.len()), group);
        for id in &ordered {
            let mut child = match by_id.get(id) {
                Some(child) => (*child).clone(),
                None => continue,
            };
            child.parent_id = Some(group_id);
            layers.push(child);
        }
        self.begin_edit("Group Layers");
        if let Some(doc) = self.document.as_mut() {
            doc.layers = layers;
        }
        self.active_layer_id = Some(group_id);
        self.selected_layer_ids = vec![group_id];
        if let Some(parent) = parent {
            self.collapsed_group_ids.remove(&parent);
        }
        self.end_edit();
        let _ = &mut names;
    }

    /// `addGroup` — a new empty "Folder N" above the active layer, parented
    /// like a sibling of it (inside it when the active layer is a folder).
    pub fn add_group(&mut self) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        if document.layers.len() >= 10_000 {
            return;
        }
        let names: std::collections::HashSet<String> =
            document.layers.iter().map(|l| l.name.clone()).collect();
        let mut number = 1;
        while names.contains(&format!("Folder {number}")) {
            number += 1;
        }
        let mut group = ImageLayer::blank(format!("Folder {number}"), document.size());
        group.is_group = true;
        group.parent_id = match self.active_layer() {
            Some(active) if active.is_group => self.active_layer_id,
            Some(active) => active.parent_id,
            None => None,
        };
        let insertion = self
            .active_layer_id
            .and_then(|id| document.layer_index(id))
            .map(|i| i + 1)
            .unwrap_or(document.layers.len());
        let group_id = group.id;
        let group_parent = group.parent_id;
        self.begin_edit("New Folder");
        if let Some(doc) = self.document.as_mut() {
            doc.layers.insert(insertion.min(doc.layers.len()), group);
            self.active_layer_id = Some(group_id);
            self.selected_layer_ids = vec![group_id];
        }
        if let Some(parent) = group_parent {
            self.collapsed_group_ids.remove(&parent);
        }
        self.end_edit();
    }

    /// `toggleGroupExpansion` — collapsing takes the active selection with it
    /// when it lives inside the folder.
    pub fn toggle_group_expansion(&mut self, id: Uuid) {
        let Some(document) = self.document.as_ref() else {
            return;
        };
        if !document.layer(id).map(|l| l.is_group).unwrap_or(false) {
            return;
        }
        if self.collapsed_group_ids.contains(&id) {
            self.collapsed_group_ids.remove(&id);
        } else {
            let active_inside = self
                .active_layer_id
                .map(|a| document.descendant_ids(id).contains(&a))
                .unwrap_or(false);
            if active_inside {
                self.select_layer(Some(id));
            }
            self.collapsed_group_ids.insert(id);
        }
    }

    /// `canPlaceLayer` — a folder never lands inside its own subtree.
    pub fn can_place_layer(&self, id: Uuid, parent: Option<Uuid>) -> bool {
        if !self.can_edit_layers() {
            return false;
        }
        let Some(document) = self.document.as_ref() else {
            return false;
        };
        if !document.layers.iter().any(|l| l.id == id) {
            return false;
        }
        let Some(parent) = parent else {
            return true;
        };
        parent != id
            && !document.descendant_ids(id).contains(&parent)
            && document.layer(parent).map(|l| l.is_group).unwrap_or(false)
    }

    /// `placeLayer` — moves a layer (with its subtree) under `parent`, above
    /// `target`, or at the very bottom, as one "Move Layer" undo step.
    pub fn place_layer(&mut self, id: Uuid, parent: Option<Uuid>, above: Option<Uuid>, at_bottom: bool) -> bool {
        if above == Some(id) || !self.can_place_layer(id, parent) {
            return false;
        }
        let Some(document) = self.document.clone() else {
            return false;
        };
        let Some(index) = document.layer_index(id) else {
            return false;
        };
        let mut layers = document.layers.clone();
        let mut layer = layers.remove(index);
        layer.parent_id = parent;
        let mut insertion = if at_bottom { 0 } else { layers.len() };
        if let Some(target) = above {
            match layers
                .iter()
                .position(|l| l.id == target && l.parent_id == parent)
            {
                Some(target_index) => insertion = target_index + 1,
                None => return false,
            }
        }
        layers.insert(insertion.min(layers.len()), layer);
        adopt_clipping(id, &mut layers);
        release_detached_clipping(&mut layers);
        self.begin_edit("Move Layer");
        if let Some(doc) = self.document.as_mut() {
            doc.layers = layers;
        }
        self.active_layer_id = Some(id);
        self.selected_layer_ids = vec![id];
        if let Some(parent) = parent {
            self.collapsed_group_ids.remove(&parent);
        }
        self.end_edit();
        true
    }

    /// `moveActiveLayerOutOfGroup` — above the group, at the group's parent.
    pub fn move_active_layer_out_of_group(&mut self) {
        let Some(layer) = self.active_layer().cloned() else {
            return;
        };
        let Some(parent) = layer.parent_id else {
            return;
        };
        let Some(group) = self.document.as_ref().and_then(|d| d.layer(parent)).cloned() else {
            return;
        };
        self.place_layer(layer.id, group.parent_id, Some(group.id), false);
    }

    /// `reorderLayers` — panel rows move top-to-bottom while the document
    /// stores bottom-to-top, so the panel order is the reversed array.
    pub fn reorder_layers(&mut self, from: &[usize], destination: usize) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.as_ref() else {
            return;
        };
        let mut layers = document.layers.clone();
        layers.reverse();
        if from.is_empty() || from.iter().any(|i| *i >= layers.len()) || destination > layers.len() {
            return;
        }
        // Array.move(fromOffsets:toOffset:): remove (lowest first), then
        // insert before the destination, adjusted for rows removed above it.
        let mut moved = Vec::new();
        let mut removed_below = 0usize;
        let mut offsets: Vec<usize> = from.to_vec();
        offsets.sort_unstable();
        offsets.dedup();
        for (k, offset) in offsets.iter().enumerate() {
            if *offset < destination {
                removed_below += 1;
            }
            moved.push(layers.remove(offset - k));
        }
        moved.reverse();
        let insertion = (destination - removed_below).min(layers.len());
        for (k, layer) in moved.into_iter().enumerate() {
            layers.insert(insertion + k, layer);
        }
        layers.reverse();
        self.begin_edit("Reorder Layers");
        if let Some(doc) = self.document.as_mut() {
            doc.layers = layers;
        }
        self.end_edit();
    }

    /// Photoshop's eye swipe (`beginVisibilitySwipe`): opens one undo step
    /// that every following swipe joins until `end_visibility_swipe`.
    pub fn begin_visibility_swipe(&mut self, id: Uuid) -> Option<bool> {
        if !self.can_edit_layers() {
            return None;
        }
        let visible = !self.document.as_ref()?.layer(id)?.is_visible;
        self.begin_edit(if visible { "Show Layer" } else { "Hide Layer" });
        self.set_visibility_in_swipe(id, visible);
        Some(visible)
    }

    pub fn set_visibility_in_swipe(&mut self, id: Uuid, visible: bool) {
        if let Some(document) = self.document.as_mut() {
            if let Some(layer) = document.layer_mut(id) {
                if layer.is_visible != visible {
                    layer.is_visible = visible;
                }
            }
        }
    }

    pub fn end_visibility_swipe(&mut self) {
        self.end_edit();
    }

    /// `deleteSelectedLayers` — every selected layer (a selected folder takes
    /// its contents) in one undo step; one selection just deletes it.
    pub fn delete_selected_layers(&mut self) {
        if !self.can_edit_layers() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let ids: Vec<Uuid> = document
            .layers
            .iter()
            .map(|l| l.id)
            .filter(|id| self.selected_layer_ids.contains(id))
            .collect();
        if ids.len() <= 1 {
            self.delete_active_layer();
            return;
        }
        let mut removed = std::collections::HashSet::new();
        for id in &ids {
            removed.extend(document.descendant_ids(*id));
        }
        self.begin_edit("Delete Layers");
        if let Some(doc) = self.document.as_mut() {
            doc.layers.retain(|l| !removed.contains(&l.id));
            for layer in doc.layers.iter_mut() {
                if layer
                    .mask_source_id
                    .map(|source| removed.contains(&source))
                    .unwrap_or(false)
                {
                    layer.mask_source_id = None;
                }
            }
        }
        if self
            .active_layer_id
            .map(|a| removed.contains(&a))
            .unwrap_or(false)
        {
            let count = self.document.as_ref().map(|d| d.layers.len()).unwrap_or(0);
            if count == 0 {
                self.active_layer_id = None;
                self.selected_layer_ids.clear();
            } else {
                let index = document
                    .layer_index(ids[0])
                    .unwrap_or(0)
                    .min(count - 1);
                let neighbor = self.document.as_ref().map(|d| d.layers[index].id);
                self.active_layer_id = neighbor;
                self.selected_layer_ids = neighbor.map(|n| vec![n]).unwrap_or_default();
            }
        }
        self.end_edit();
    }

    /// `layerRows` — the Layers-panel rows, top first, honoring collapse.
    pub fn layer_rows(&self) -> Vec<crate::document::HierarchyEntry<'_>> {
        match self.document.as_ref() {
            Some(document) => document.hierarchy_entries(true, &self.collapsed_group_ids),
            None => Vec::new(),
        }
    }

    // MARK: Import pipeline (EditorSession+Import / ImageImporter)

    /// `insert(_:centeredAt:)` — inserts pixels as a new top layer centered on
    /// `point` (the canvas center without one). With no document the image
    /// determines the canvas size.
    pub fn insert_image(&mut self, asset: ImportedImage, center: Option<CGPoint>) {
        if self.document.is_none() {
            self.document = Some(CanvasDocument::new(
                asset.image.width.max(1),
                asset.image.height.max(1),
            ));
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let center = center.unwrap_or(CGPoint::new(
            document.width as f64 / 2.0,
            document.height as f64 / 2.0,
        ));
        let (asset_width, asset_height) = (asset.image.width as f64, asset.image.height as f64);
        let mut layer = ImageLayer::with_asset(
            asset,
            CGPoint::new(
                (center.x - asset_width / 2.0).floor(),
                (center.y - asset_height / 2.0).floor(),
            ),
        );
        layer.parent_id = match self.active_layer() {
            Some(active) if active.is_group => self.active_layer_id,
            Some(active) => active.parent_id,
            None => None,
        };
        let layer_id = layer.id;
        self.begin_edit("Import Image");
        if let Some(doc) = self.document.as_mut() {
            doc.layers.push(layer);
        }
        self.active_layer_id = Some(layer_id);
        self.selected_layer_ids = vec![layer_id];
        self.end_edit();
    }

    /// Size guards for a bitmap import: 30,000 pixels per side, and the
    /// document's total pixel budget of 100 megapixels
    /// (`ImageImportError.tooLarge`).
    fn import_size_allowed(&self, width: usize, height: usize) -> bool {
        if width == 0 || height == 0 || width > 30_000 || height > 30_000 {
            return false;
        }
        let used: u64 = self
            .document
            .as_ref()
            .map(|d| {
                d.layers
                    .iter()
                    .filter_map(|l| l.asset.as_ref())
                    .map(|a| a.image.width as u64 * a.image.height as u64)
                    .sum()
            })
            .unwrap_or(0);
        used + (width as u64 * height as u64) <= 100_000_000
    }

    /// `importImages`/`drainImports` for bitmaps: decodes each file, applies
    /// the size guards, and inserts every success centered on `point` (which
    /// is ignored while there is no document — the first image determines the
    /// canvas). Returns one failure line per rejected file.
    pub fn import_images(
        &mut self,
        paths: &[std::path::PathBuf],
        center: Option<CGPoint>,
    ) -> Vec<String> {
        if paths.is_empty() {
            return Vec::new();
        }
        self.commit_transform();
        self.cancel_crop();
        self.begin_edit("Import Images");
        let mut failures = Vec::new();
        for path in paths {
            let name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_else(|| "Image".to_string());
            let Some((bitmap, _thumbnail)) = decode_image(path) else {
                failures.push(format!("{}: the image could not be read.", path.display()));
                continue;
            };
            if !self.import_size_allowed(bitmap.width, bitmap.height) {
                failures.push(format!(
                    "{}: the image is too large for the document.",
                    path.display()
                ));
                continue;
            }
            // No document yet: the drop point does not apply, the first image
            // determines the canvas.
            let point = if self.document.is_some() { center } else { None };
            let asset = ImportedImage::new(
                Bitmap::from_raw(bitmap.width, bitmap.height, bitmap.data),
                &name,
            );
            self.insert_image(asset, point);
        }
        self.end_edit();
        failures
    }

    /// `insertPhotoshop` for file data: parses the PSD, applies it as a fresh
    /// document (no document yet) or as a wrapping group, and reports how many
    /// conversion notes the import produced.
    pub fn import_photoshop_data(&mut self, name: &str, data: &[u8]) -> Result<usize, String> {
        let parsed = crate::psd::PsdReader::read_with_budget(data, 100_000_000)
            .map_err(|error| error.to_string())?;
        let assets = crate::psd::PsdDocumentBuilder::assets(&parsed);
        let imported = crate::psd::PsdDocumentBuilder::make_import_with_assets(&parsed, &assets);
        let mut incoming = imported.layers;
        let wrapping = self.document.is_some();
        let added = incoming.len() + usize::from(wrapping);
        let layer_count = self.document.as_ref().map(|d| d.layers.len()).unwrap_or(0);
        if layer_count + added > 10_000 {
            return Err("too many layers".to_string());
        }
        self.begin_edit("Import Photoshop File");
        if !wrapping {
            let mut document = CanvasDocument::new(imported.width, imported.height);
            document.resolution = imported.resolution;
            let active = incoming
                .iter()
                .rev()
                .find(|layer| layer.parent_id.is_none())
                .map(|layer| layer.id)
                .or_else(|| incoming.last().map(|layer| layer.id));
            document.layers = incoming;
            self.document = Some(document);
            self.select_layer(active);
        } else {
            let size = self.document.as_ref().unwrap().size();
            let mut group = ImageLayer::blank(name.to_string(), size);
            group.is_group = true;
            group.parent_id = match self.active_layer() {
                Some(active) if active.is_group => self.active_layer_id,
                Some(active) => active.parent_id,
                None => None,
            };
            let group_id = group.id;
            for layer in incoming.iter_mut() {
                if layer.parent_id.is_none() {
                    layer.parent_id = Some(group_id);
                }
            }
            if let Some(doc) = self.document.as_mut() {
                doc.layers.push(group);
                doc.layers.extend(incoming);
            }
            self.active_layer_id = Some(group_id);
            self.selected_layer_ids = vec![group_id];
        }
        self.end_edit();
        Ok(imported.conversions.len())
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
            let mut source_of = |id: Uuid, document: &CanvasDocument| -> Option<Uuid> {
                if extra.map(|(target, source)| target == id).unwrap_or(false) {
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
        if self.document.as_ref().and_then(|d| d.layer(target)).and_then(|l| l.mask_source_id) == Some(source) {
            return true;
        }
        self.begin_edit("Create Clipping Mask");
        if let Some(document) = self.document.as_mut() {
            if let Some(layer) = document.layer_mut(target) {
                layer.mask_source_id = Some(source);
            }
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
            let keeps = *id == target
                || document.layer(*id).and_then(|l| l.mask_source_id) == Some(source);
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
            crate::render::draw_layer_pixels(
                asset.image.as_ref(),
                &source.transform,
                &mut placed,
            );
            // A raster mask on the source multiplies in, as the bake's
            // `LayerRenderer.draw` does.
            if let Some(mask) = &source.mask {
                if mask.is_enabled {
                    if let Some(grid) =
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
                    && l.mask_source_id.map(|s| removed.contains(&s)).unwrap_or(false)
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
        self.delete_selected_layers_with_baked(&ids.to_vec(), baked);
        true
    }

    /// `finishDeletingLayers` — the deletion that carries baked pixels for
    /// dependents.
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
        self.delete_selected_layers();
        if let Some(document) = self.document.as_mut() {
            for (id, image) in baked {
                if let Some(layer) = document.layer_mut(id) {
                    layer.asset = Some(image);
                }
            }
        }
    }

    // MARK: Transform snapping (Guides.swift)

    /// `alignmentSnapTargets` — the canvas bounds and other layers' edges
    /// (plus centers on request), rounded to whole pixels.
    pub fn alignment_snap_targets(
        &self,
        excluding: &[Uuid],
        include_centers: bool,
    ) -> (Vec<CGFloat>, Vec<CGFloat>) {
        let Some(document) = self.document.as_ref() else {
            return (Vec::new(), Vec::new());
        };
        if !self.snapping_enabled {
            return (Vec::new(), Vec::new());
        }
        let mut xs = vec![0.0, document.width as CGFloat];
        let mut ys = vec![0.0, document.height as CGFloat];
        if include_centers {
            xs.push(document.width as CGFloat / 2.0);
            ys.push(document.height as CGFloat / 2.0);
        }
        for layer in document.render_layers() {
            if layer.asset.is_none() || excluding.contains(&layer.id) {
                continue;
            }
            let corners = layer.transform.corners();
            let min_x = corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min).round();
            let max_x = corners.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max).round();
            let min_y = corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min).round();
            let max_y = corners.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max).round();
            xs.push(min_x);
            xs.push(max_x);
            ys.push(min_y);
            ys.push(max_y);
            if include_centers {
                xs.push(((min_x + max_x) / 2.0).round());
                ys.push(((min_y + max_y) / 2.0).round());
            }
        }
        (xs, ys)
    }

    /// `transformSnapTargets` — moving layers snap to edges and centers.
    pub fn transform_snap_targets(&self, excluding: &[Uuid]) -> (Vec<CGFloat>, Vec<CGFloat>) {
        self.alignment_snap_targets(excluding, true)
    }

    /// `snappedMove` — `draft` nudged so the layer it places lines up with a
    /// nearby edge or center; `moving` is what is being dragged, and
    /// `tolerance` is in document pixels. The guides it landed on are stored
    /// for the canvas to draw.
    pub fn snapped_move(
        &mut self,
        draft: LayerTransform,
        moving: &[Uuid],
        tolerance: CGFloat,
    ) -> LayerTransform {
        if !self.snapping_enabled {
            self.snap_guides = (Vec::new(), Vec::new());
            return draft;
        }
        let corners = draft.corners();
        let min_x = corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
        let max_x = corners.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
        let min_y = corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let max_y = corners.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max);
        let bounds = crate::geometry::CGRect::new(min_x, min_y, max_x - min_x, max_y - min_y);
        let targets = self.transform_snap_targets(moving);
        let (offset, guide_x, guide_y) =
            crate::transform::TransformSnap::offset(&bounds, &targets.0, &targets.1, tolerance);
        self.snap_guides = (
            guide_x.map(|g| vec![g]).unwrap_or_default(),
            guide_y.map(|g| vec![g]).unwrap_or_default(),
        );
        if offset.width == 0.0 && offset.height == 0.0 {
            return draft;
        }
        let mut snapped = draft;
        snapped.origin.x += offset.width;
        snapped.origin.y += offset.height;
        snapped
    }

    // MARK: Distort (Distort.swift)

    /// Cmd-drag on a transform handle: the corners start moving freely
    /// (`beginDistort`). The edit then waits for Apply rather than applying on
    /// mouse-up.
    pub fn begin_distort(&mut self) {
        let Some(mut edit) = self.transform_edit.clone() else {
            return;
        };
        if edit.corners.is_some() || !edit.draft.is_valid() {
            return;
        }
        edit.corners = Some(crate::tools::distort::corners_of(&edit.draft));
        self.transform_edit = Some(edit);
    }

    /// Moves the distortion's corners; a twisted or collapsed shape is ignored.
    pub fn preview_corners(&mut self, corners: [CGPoint; 4]) {
        if self.transform_edit.as_ref().and_then(|edit| edit.corners).is_none()
            || !crate::tools::distort::is_usable(&corners)
        {
            return;
        }
        if let Some(edit) = self.transform_edit.as_mut() {
            edit.corners = Some(corners);
        }
    }

    /// `distortTarget` — where a distortion takes `layer`: its transform under
    /// the edit and the corners that transform moves to.
    fn distort_target(
        &self,
        layer: &ImageLayer,
        edit: &TransformEdit,
        shape: &[CGPoint; 4],
    ) -> Option<(LayerTransform, [CGPoint; 4])> {
        let _ = layer;
        if edit.group.is_none() {
            return if edit.layer_id == layer.id {
                Some((edit.draft, *shape))
            } else {
                None
            };
        }
        let group = edit.group.as_ref()?;
        let original = group.original(layer.id)?;
        let transform = original.following(&group.box_, &edit.draft);
        let corners = crate::tools::distort::carried(&transform, &edit.draft, shape);
        if !crate::tools::distort::is_usable(&corners) {
            return None;
        }
        Some((transform, corners))
    }

    /// `commitDistort` — each distorted layer's pixels and mask are resampled
    /// into its shape, as one undo step.
    fn commit_distort(&mut self, edit: &TransformEdit, shape: [CGPoint; 4]) {
        let ids: Vec<Uuid> = match &edit.group {
            Some(group) => group.originals.iter().map(|(id, _)| *id).collect(),
            None => vec![edit.layer_id],
        };
        self.begin_edit(if edit.group.is_none() { "Distort" } else { "Distort Layers" });
        for id in ids {
            let Some(document) = self.document.as_ref() else {
                continue;
            };
            let Some(layer) = document.layer(id).cloned() else {
                continue;
            };
            let Some((transform, corners)) = self.distort_target(&layer, edit, &shape) else {
                continue;
            };
            self.distort_at(id, &transform, &corners);
        }
        self.end_edit();
    }

    /// `distort(at:transform:corners:)` — pixels warp into the shape (cropped
    /// to what is visible); a linked mask warps with the pixels, a linked
    /// mask placed apart carries the same perspective over its own bounds,
    /// and an unlinked mask stays put.
    fn distort_at(&mut self, id: Uuid, transform: &LayerTransform, corners: &[CGPoint; 4]) {
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(layer) = document.layer(id).cloned() else {
            return;
        };
        let Some(image) = layer.asset.as_ref().map(|a| a.image.as_ref().clone()) else {
            return;
        };
        let Ok((warped, placed, crop)) =
            crate::tools::distort::warp_trimmed(&image, transform, corners)
        else {
            return;
        };
        let mut mask = layer.mask.clone();
        if let Some(original) = layer.mask.clone() {
            if original.placement.is_none() && original.is_linked {
                // Warp with the pixels, then crop with them.
                if let Ok((warped_mask, _)) = crate::tools::distort::warp(
                    original.asset.image.as_ref(),
                    transform,
                    corners,
                    true,
                    None,
                ) {
                    if warped_mask.width == 1 && warped_mask.height == 1 {
                        // A uniform mask passes through.
                    } else {
                        let crop_x = crop.min_x().round().max(0.0) as usize;
                        let crop_y = crop.min_y().round().max(0.0) as usize;
                        let crop_w = crop.width().round().max(1.0) as usize;
                        let crop_h = crop.height().round().max(1.0) as usize;
                        let mut cropped = Bitmap::new(crop_w.min(warped_mask.width - crop_x.min(warped_mask.width)), crop_h.min(warped_mask.height - crop_y.min(warped_mask.height)));
                        for y in 0..cropped.height {
                            for x in 0..cropped.width {
                                cropped.set_pixel(x, y, warped_mask.pixel(crop_x + x, crop_y + y));
                            }
                        }
                        mask = Some(original.replacing(ImportedImage::new(cropped, "Layer Mask")));
                    }
                }
            } else if original.is_linked {
                if let Some(placed_mask) = original.placement {
                    let placement = placed_mask.following(&layer.transform, transform);
                    let carried = crate::tools::distort::carried(&placement, transform, corners);
                    if crate::tools::distort::is_convex(&carried) {
                        let background = LayerMask::background_of(&original.asset.thumbnail);
                        if let Ok((moved, moved_transform)) = crate::tools::distort::warp_mask(
                            original.asset.image.as_ref(),
                            &placement,
                            &carried,
                            background,
                            None,
                        ) {
                            let mut updated = original.clone();
                            updated.asset = ImportedImage::new(moved, "Layer Mask");
                            updated.placement = Some(moved_transform);
                            mask = Some(updated);
                        }
                    } else {
                        mask.as_mut().unwrap().placement = Some(placement);
                    }
                }
            } else {
                // An unlinked mask keeps its place on the document.
                if let Some(mask_ref) = mask.as_mut() {
                    mask_ref.placement = original.placement.or(Some(layer.transform));
                }
            }
        }
        if let Some(document) = self.document.as_mut() {
            if let Some(current) = document.layer_mut(id) {
                current.asset = Some(ImportedImage::new(warped, current.name.clone()));
                current.transform = placed;
                current.mask = mask;
            }
        }
    }

    /// `commitMaskTransform`'s distortion branch: a distorted mask resamples
    /// into its shape over the layer's bounds ("Distort Layer Mask").
    fn commit_mask_distort(&mut self, edit: &TransformEdit, shape: [CGPoint; 4]) {
        if !edit.draft.is_valid() {
            return;
        }
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(layer) = document.layer(edit.layer_id).cloned() else {
            return;
        };
        let Some(mask) = layer.mask.clone() else {
            return;
        };
        let background = LayerMask::background_of(&mask.asset.thumbnail);
        let Some((moved, moved_transform)) = crate::tools::distort::warp_mask(
            mask.asset.image.as_ref(),
            &edit.draft,
            &shape,
            background,
            None,
        )
        .ok() else {
            return;
        };
        let placement = if moved_transform.same_placement(&layer.transform) {
            None
        } else {
            Some(moved_transform)
        };
        self.begin_edit("Distort Layer Mask");
        if let Some(doc) = self.document.as_mut() {
            if let Some(current) = doc.layer_mut(edit.layer_id) {
                if let Some(current_mask) = current.mask.as_mut() {
                    current_mask.asset = ImportedImage::new(moved, "Layer Mask");
                    current_mask.placement = placement;
                }
            }
        }
        self.end_edit();
    }

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
                    visible.contains(&l.id) && l.live_text().is_some() && l.transform.contains(point)
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
            origin: target_layer
                .as_ref()
                .map(|l| l.origin())
                .unwrap_or(point),
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
            (rect.width().round() as f64).max(16.0),
            (rect.height().round() as f64).max(16.0),
        ));
        if !style.box_is_valid() {
            self.brush_error = Some("That text box exceeds the 30,000-pixel or 100-megapixel limit.".into());
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
        if self.document.as_ref().map(|d| d.id) != Some(draft.document_id) || !draft.style.is_valid() {
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
        if draft.layer_id.is_none()
            && draft.style.content.trim().is_empty()
        {
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
            let Some(index) = document.layer_index(layer_id) else {
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
            if let Some(doc) = self.document.as_mut() {
                if let Some(current) = doc.layer_mut(layer_id) {
                    let name = asset.name.clone();
                    let mask_placement = current
                        .mask
                        .as_ref()
                        .filter(|m| m.placement.is_none())
                        .map(|_| current.mask_transform());
                    if let Some(placement) = mask_placement {
                        if let Some(mask) = current.mask.as_mut() {
                            mask.placement = Some(placement);
                        }
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
        if let Some(document) = self.document.as_mut() {
            if let Some(current) = document.layer_mut(id) {
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
            .or_else(|| self.active_layer().and_then(|l| l.live_text().map(|text| text.style.clone())))
            .unwrap_or_else(|| self.text_defaults.clone())
    }

    /// `changeTextStyle` — edits the pending draft, the active live-text
    /// layer (starting a draft), or the defaults.
    pub fn change_text_style(&mut self, change: impl FnOnce(&mut crate::document::LayerTextStyle)) {
        if self.text_draft.is_none() && self.active_layer().and_then(|l| l.live_text()).is_some() {
            self.edit_active_text();
        }
        if let Some(draft) = self.text_draft.as_mut() {
            change(&mut draft.style);
            if !draft.style.is_valid() {
                return;
            }
        } else {
            let mut style = self.text_defaults.clone();
            change(&mut style);
            if style.is_valid() {
                self.text_defaults = style;
            }
        }
    }

    // MARK: Transforms

    pub fn commit_transform(&mut self) {
        self.finish_opacity_edit();
        let Some(edit) = self.transform_edit.take() else {
            return;
        };
        // Cmd-T on a selection: unchanged drafts restore exactly (so soft
        // selection edges never pick up a seam); anything else merges.
        if let Some(floating) = edit.floating.clone() {
            if edit.draft == floating.original && edit.corners.is_none() {
                self.cancel_floating_transform(&floating);
            } else {
                self.merge_floating_transform(&edit, &floating);
            }
            return;
        }
        // A distortion (Cmd-dragged handles): pixels and mask resample into
        // the shape, as one undo step (commitDistort).
        if let Some(shape) = edit.corners {
            if edit.mask {
                self.commit_mask_distort(&edit, shape);
            } else {
                self.commit_distort(&edit, shape);
            }
            return;
        }
        // An unlinked mask transformed on its own takes the new placement;
        // its layer stays put (commitMaskTransform).
        if edit.mask {
            self.commit_mask_transform(&edit);
            return;
        }
        if !edit.draft.is_valid() {
            return;
        }
        let Some(document) = self.document.as_mut() else {
            return;
        };
        let Some(index) = document.layer_index(edit.layer_id) else {
            return;
        };
        self.begin_edit("Transform Layer");
        let document = self.document.as_mut().unwrap();
        let layer = &mut document.layers[index];
        if let Some(mask) = layer.mask.as_mut() {
            let old = layer.transform;
            mask.placement = mask.placement_moving_layer(&old, &edit.draft);
        }
        layer.transform = edit.draft;
        self.end_edit();
    }

    /// `commitMaskTransform` — the mask alone moved: its pixels are untouched,
    /// it just takes the placement, and the undo step is named for the mask.
    fn commit_mask_transform(&mut self, edit: &TransformEdit) {
        if !edit.draft.is_valid() {
            return;
        }
        let placement = self.document.as_ref().and_then(|document| {
            let layer = document.layer(edit.layer_id)?;
            let mask = layer.mask.as_ref()?;
            Some(if edit.draft.same_placement(&layer.transform) {
                None
            } else {
                Some(edit.draft)
            })
            .filter(|placement| *placement != mask.placement)
        });
        let Some(placement) = placement else {
            return;
        };
        self.begin_edit("Transform Layer Mask");
        if let Some(doc) = self.document.as_mut() {
            if let Some(layer) = doc.layer_mut(edit.layer_id) {
                if let Some(mask) = layer.mask.as_mut() {
                    mask.placement = placement;
                }
            }
        }
        self.end_edit();
    }

    /// `canTransformSelection` — Cmd-T with a selection floats the selected
    /// pixels instead of transforming the layer.
    pub fn can_transform_selection(&self) -> bool {
        self.transform_edit.is_none()
            && self.can_edit_pixels()
            && !self.is_mask_selected
            && self.selection().map(|s| !s.is_empty()).unwrap_or(false)
            && self.active_layer().and_then(|l| l.asset.as_ref()).is_some()
    }

    /// `beginSelectionTransform`: the selected pixels lift onto a temporary
    /// "Floating Selection" layer above their source — with the source's
    /// parent, opacity and blend mode — and the whole thing opens one
    /// "Transform Selection" undo step that merge/cancel closes.
    pub fn begin_selection_transform(&mut self) {
        if !self.can_transform_selection() {
            return;
        }
        let Some(source) = self.active_layer().cloned() else {
            return;
        };
        let Some((lifted_image, region)) = self.render_selected_pixels(false) else {
            return;
        };
        let Some(before) = self.document.clone() else {
            return;
        };
        let before_active = self.active_layer_id;
        self.begin_edit("Transform Selection");
        self.clear_selected_pixels();
        let mut floating =
            ImageLayer::with_asset(ImportedImage::new(lifted_image, "Floating Selection"), region.origin);
        floating.parent_id = source.parent_id;
        floating.opacity = source.opacity;
        floating.blend_mode = source.blend_mode;
        let floating_id = floating.id;
        let original = floating.transform;
        if let Some(doc) = self.document.as_mut() {
            let index = doc
                .layer_index(source.id)
                .map(|i| i + 1)
                .unwrap_or(doc.layers.len());
            doc.layers.insert(index.min(doc.layers.len()), floating);
        }
        self.active_layer_id = Some(floating_id);
        self.tool = NavigationTool::Move;
        self.transform_edit = Some(TransformEdit {
            layer_id: floating_id,
            draft: original,
            persistent: true,
            floating: Some(crate::selection::FloatingTransform {
                source_id: source.id,
                before: Box::new(before),
                before_active,
                original,
                pixel_size: CGSize::new(region.width(), region.height()),
            }),
            corners: None,
            mask: false,
            group: None,
        });
    }

    /// `floatingSelectionTransform` — maps the original selection to where the
    /// floating pixels are now.
    pub fn floating_selection_transform(edit: &TransformEdit) -> Option<crate::geometry::CGAffineTransform> {
        let floating = edit.floating.as_ref()?;
        let width = floating.pixel_size.width;
        let height = floating.pixel_size.height;
        // Applied to a document point: undo the original placement, then the
        // current draft.
        Some(
            crate::raster::pixel_to_document(&edit.draft, width, height)
                .concat(&crate::raster::pixel_to_document(&floating.original, width, height).inverted()),
        )
    }

    /// `mergeFloatingTransform`: composites the transformed pixels back into
    /// their layer, moves the selection with them, and closes the undo step.
    fn merge_floating_transform(&mut self, edit: &TransformEdit, floating: &crate::selection::FloatingTransform) {
        let merged = self.merge_floating_into_source(edit, floating);
        match merged {
            Some(()) => {
                self.end_edit();
            }
            None => {
                // Nothing partial survives a failed merge.
                self.document = Some((*floating.before).clone());
                self.active_layer_id = floating.before_active;
                self.brush_error = Some("The floating selection could not be merged.".into());
                self.end_edit();
            }
        }
    }

    fn merge_floating_into_source(
        &mut self,
        edit: &TransformEdit,
        floating: &crate::selection::FloatingTransform,
    ) -> Option<()> {
        if !edit.draft.is_valid() {
            return None;
        }
        let document = self.document.as_ref()?;
        let pixels = document
            .layer(edit.layer_id)
            .and_then(|l| l.asset.as_ref())
            .map(|a| a.image.as_ref().clone())?;
        let source = document.layer(floating.source_id).cloned()?;
        let (asset, merged_transform, merged_mask) = floating_merge(&pixels, &edit.draft, &source)?;
        // The selection moves with the pixels.
        let moved = self.selection().and_then(|selection| {
            let matrix = Self::floating_selection_transform(edit)?;
            Some(crate::selection::DocumentSelection {
                path: selection.path.transformed(&matrix),
                antialiased: selection.antialiased,
                feather: selection.feather,
            })
        });
        let document = self.document.as_mut()?;
        document.layers.retain(|l| l.id != edit.layer_id);
        let layer = document.layer_mut(floating.source_id)?;
        layer.asset = Some(asset);
        layer.transform = merged_transform;
        layer.mask = merged_mask;
        document.selection = moved;
        self.active_layer_id = Some(floating.source_id);
        Some(())
    }

    /// `cancelFloatingTransform` — Escape restores the document exactly.
    pub fn cancel_floating_transform(&mut self, floating: &crate::selection::FloatingTransform) {
        self.document = Some((*floating.before).clone());
        self.active_layer_id = floating.before_active;
        self.end_edit();
    }

    pub fn cancel_transform(&mut self) {
        let Some(edit) = self.transform_edit.take() else {
            return;
        };
        if let Some(floating) = edit.floating {
            self.cancel_floating_transform(&floating);
        }
    }

    pub fn begin_transform(&mut self) {
        self.cancel_crop();
        if self.transform_edit.is_some() {
            return;
        }
        if !self.can_transform() {
            return;
        }
        let Some(layer) = self.active_layer().cloned() else {
            return;
        };
        self.tool = NavigationTool::Move;
        let mask_alone = self
            .is_mask_selected
            && layer.mask.as_ref().map(|m| !m.is_linked).unwrap_or(false);
        let draft = if mask_alone {
            layer.mask_transform()
        } else {
            layer.transform
        };
        self.transform_edit = Some(TransformEdit {
            layer_id: layer.id,
            draft,
            persistent: true,
            floating: None,
            corners: None,
            mask: mask_alone,
            group: None,
        });
    }

    pub fn preview_transform(&mut self, value: LayerTransform) {
        if value.is_valid() {
            if let Some(edit) = self.transform_edit.as_mut() {
                edit.draft = value;
            }
        }
    }

    pub fn can_transform(&self) -> bool {
        let Some(layer) = self.active_layer() else {
            return false;
        };
        layer.asset.is_some() && !layer.is_group
    }

    pub fn nudge_layer(&mut self, dx: f64, dy: f64) {
        let already = self.transform_edit.is_some();
        if !already {
            self.begin_transform();
        }
        if let Some(edit) = self.transform_edit.as_mut() {
            let mut value = edit.draft;
            value.origin.x += dx;
            value.origin.y += dy;
            self.preview_transform(value);
        }
        if !already {
            self.commit_transform();
        }
    }

    /// Flips the selected layer about its own middle.
    pub fn flip_layers(&mut self, horizontally: bool) {
        self.commit_transform();
        if !self.can_transform() {
            return;
        }
        let Some(id) = self.active_layer_id else {
            return;
        };
        let axis = self
            .active_layer()
            .map(|l| if horizontally { l.transform.center().x } else { l.transform.center().y });
        let Some(axis) = axis else {
            return;
        };
        self.finish_opacity_edit();
        self.begin_edit(if horizontally { "Flip Horizontal" } else { "Flip Vertical" });
        if let Some(doc) = self.document.as_mut() {
            if let Some(layer) = doc.layer_mut(id) {
                let flipped = mirrored_transform(&layer.transform, horizontally, axis);
                if let Some(mask) = layer.mask.as_mut() {
                    let old = layer.transform;
                    mask.placement = mask.placement_moving_layer(&old, &flipped);
                }
                layer.transform = flipped;
            }
        }
        self.end_edit();
    }

    /// Flips the whole canvas.
    pub fn flip_canvas(&mut self, horizontally: bool) {
        self.commit_transform();
        self.cancel_crop();
        let Some(document) = self.document.clone() else {
            return;
        };
        let axis = if horizontally {
            document.size().width / 2.0
        } else {
            document.size().height / 2.0
        };
        self.finish_opacity_edit();
        self.begin_edit(if horizontally {
            "Flip Canvas Horizontal"
        } else {
            "Flip Canvas Vertical"
        });
        if let Some(doc) = self.document.as_mut() {
            for layer in doc.layers.iter_mut() {
                layer.transform = mirrored_transform(&layer.transform, horizontally, axis);
                if let Some(mask) = layer.mask.as_mut() {
                    if let Some(placement) = mask.placement.as_mut() {
                        *placement = mirrored_transform(placement, horizontally, axis);
                    }
                }
            }
            doc.guides = doc
                .guides
                .iter()
                .map(|g| g.mirrored(horizontally, axis))
                .collect();
        }
        self.end_edit();
    }

    // MARK: Merge

    /// ⌘E: merges as the canvas shows them — blend modes, opacity, masks,
    /// clipping and adjustments baked in — into one pixel layer.
    pub fn merge_layers(&mut self) {
        self.commit_transform();
        let Some(document) = self.document.clone() else {
            return;
        };
        let Some(active) = self.active_layer().cloned() else {
            return;
        };
        let (ids, action, name): (Vec<Uuid>, &'static str, String) =
            if self.selected_layer_ids.len() > 1 {
                let mut picked: Vec<Uuid> = Vec::new();
                for id in &self.selected_layer_ids {
                    picked.extend(document.descendant_ids(*id));
                }
                let ordered: Vec<Uuid> = document
                    .layers
                    .iter()
                    .filter(|l| picked.contains(&l.id))
                    .map(|l| l.id)
                    .collect();
                let name = document
                    .layers
                    .iter()
                    .rev()
                    .find(|l| self.selected_layer_ids.contains(&l.id))
                    .map(|l| l.name.clone())
                    .unwrap_or_default();
                (ordered, "Merge Layers", name)
            } else if active.is_group {
                let inside = document.descendant_ids(active.id);
                let mut ids: Vec<Uuid> = document
                    .layers
                    .iter()
                    .filter(|l| inside.contains(&l.id))
                    .map(|l| l.id)
                    .collect();
                ids.push(active.id);
                let name = active.name.clone();
                (ids, "Merge Group", name)
            } else {
                let Some(index) = document.layer_index(active.id) else {
                    return;
                };
                let below = document.layers[..index]
                    .iter()
                    .rev()
                    .find(|l| l.parent_id == active.parent_id && !l.is_group)
                    .cloned();
                let Some(below) = below else {
                    return;
                };
                (vec![below.id, active.id], "Merge Down", below.name.clone())
            };
        let kept: std::collections::HashSet<Uuid> = ids.iter().copied().collect();

        // Composite the subset onto a canvas.
        let mut subset_doc = document.clone();
        subset_doc.layers = document
            .layers
            .iter()
            .filter(|l| kept.contains(&l.id))
            .cloned()
            .collect();
        for layer in subset_doc.layers.iter_mut() {
            if let Some(parent) = layer.parent_id {
                if !kept.contains(&parent) {
                    layer.parent_id = None;
                }
            }
            if let Some(source) = layer.mask_source_id {
                if !kept.contains(&source) {
                    layer.mask_source_id = None;
                }
            }
        }
        let full = crate::render::draw_live_composite(&subset_doc);
        let canvas_transform = LayerTransform::new(
            CGPoint::ZERO,
            CGSize::new(document.width as f64, document.height as f64),
        );
        let (trimmed_image, trimmed_transform) = crate::filters::trimmed(&full, canvas_transform);
        let anchor = *ids.last().unwrap_or(&active.id);
        let parent = document.layer(anchor).and_then(|l| l.parent_id);
        let mut merged = ImageLayer::with_asset(
            ImportedImage::new(trimmed_image, name.clone()),
            trimmed_transform.origin,
        );
        merged.transform = trimmed_transform;
        merged.parent_id = parent;

        let mut next: Vec<ImageLayer> = document
            .layers
            .iter()
            .filter(|l| !kept.contains(&l.id))
            .cloned()
            .collect();
        for layer in next.iter_mut() {
            if let Some(source) = layer.mask_source_id {
                if kept.contains(&source) {
                    layer.mask_source_id = Some(merged.id);
                }
            }
        }
        let slot = document.layer_index(anchor).unwrap_or(document.layers.len());
        let insertion =
            slot - document.layers[..slot].iter().filter(|l| kept.contains(&l.id)).count();
        let insertion = insertion.min(next.len());
        let merged_id = merged.id;
        next.insert(insertion, merged);

        self.begin_edit(action);
        if let Some(doc) = self.document.as_mut() {
            doc.layers = next;
            self.active_layer_id = Some(merged_id);
            self.selected_layer_ids = vec![merged_id];
        }
        self.end_edit();
    }

    // MARK: Guides

    pub fn clear_guides(&mut self) {
        if self
            .document
            .as_ref()
            .map(|d| !d.guides.is_empty())
            .unwrap_or(false)
        {
            self.begin_edit("Clear Guides");
            if let Some(doc) = self.document.as_mut() {
                doc.guides.clear();
            }
            self.end_edit();
        }
    }

    pub fn add_guide(&mut self, axis: GuideAxis, position: f64) {
        self.shows_guides = true;
        self.begin_edit("New Guide");
        if let Some(doc) = self.document.as_mut() {
            doc.guides.push(CanvasGuide::new(axis, position));
        }
        self.end_edit();
    }

    // MARK: Crop

    pub fn cancel_crop(&mut self) {
        self.crop_rect = None;
        self.crop_drag = None;
    }

    pub fn begin_crop_drag(&mut self, start: CGPoint, mode: CropDragMode) {
        let original = self.crop_rect.unwrap_or_else(|| {
            CGRect::from_origin_size(
                CGPoint::ZERO,
                self.document_size().unwrap_or(CGSize::ZERO),
            )
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
            #[cfg(test)]
            if std::env::var("CROP_DBG").is_ok() {
                eprintln!("DBG snap in={:?} out={:?} xs={:?} ys={:?}", (updated.min_x(), updated.min_y()), (updated.min_x(), updated.min_y()), snap.xs, snap.ys);
            }
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
            let min_x = corners.iter().map(|p| p.x).fold(f64::INFINITY, f64::min).round();
            let max_x = corners.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max).round();
            let min_y = corners.iter().map(|p| p.y).fold(f64::INFINITY, f64::min).round();
            let max_y = corners.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max).round();
            xs.push(min_x);
            xs.push(max_x);
            ys.push(min_y);
            ys.push(max_y);
        }
        (xs, ys)
    }

    pub fn end_crop_drag(&mut self) {
        self.crop_drag = None;
        if let Some(rect) = self.crop_rect {
            if !CropGeometry::valid(rect) {
                self.crop_rect = None;
            }
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
        self.begin_edit("Crop");
        if let Some(doc) = self.document.take() {
            self.document = Some(resize_canvas(&doc, &options));
        }
        self.end_edit();
        self.crop_rect = None;
    }

    // MARK: Canvas / Image size, trim

    pub fn resize_canvas_to(&mut self, options: &CanvasSizeOptions) {
        self.begin_edit("Canvas Size");
        if let Some(doc) = self.document.take() {
            self.document = Some(resize_canvas(&doc, options));
        }
        self.end_edit();
    }

    pub fn resize_image_to(&mut self, width: usize, height: usize, resolution: Option<f64>) {
        if !(1..=30_000).contains(&width) || !(1..=30_000).contains(&height) {
            return;
        }
        self.begin_edit("Image Size");
        if let Some(doc) = self.document.take() {
            self.document = Some(resize_image(&doc, width, height, resolution));
        }
        self.end_edit();
    }

    /// Returns false when nothing remained to trim.
    pub fn trim(&mut self, options: &TrimOptions) -> bool {
        if let Some(doc) = self.document.clone() {
            if let Some(trimmed) = trim_document(&doc, options) {
                self.begin_edit("Trim");
                self.document = Some(trimmed);
                self.end_edit();
                return true;
            }
        }
        false
    }

    // MARK: Fills & palette

    pub fn palette_color(&self, background: bool) -> PaletteColor {
        if self.is_mask_selected {
            let white = if background {
                !self.mask_paint_white
            } else {
                self.mask_paint_white
            };
            if white {
                PaletteColor::WHITE
            } else {
                PaletteColor::BLACK
            }
        } else if background {
            self.background_color
        } else {
            PaletteColor::new(
                self.brush_settings.red,
                self.brush_settings.green,
                self.brush_settings.blue,
            )
        }
    }

    pub fn swap_palette_colors(&mut self) {
        if self.is_mask_selected {
            self.mask_paint_white = !self.mask_paint_white;
        } else {
            let old = self.palette_color(false);
            let background = self.background_color;
            self.brush_settings.red = background.red;
            self.brush_settings.green = background.green;
            self.brush_settings.blue = background.blue;
            self.background_color = old;
        }
    }

    pub fn reset_palette_colors(&mut self) {
        if self.is_mask_selected {
            self.mask_paint_white = false;
        } else {
            self.brush_settings.red = 0.0;
            self.brush_settings.green = 0.0;
            self.brush_settings.blue = 0.0;
            self.background_color = PaletteColor::WHITE;
        }
    }
}

/// `FloatingMerge.merge`: draws the floating pixels (with their transform)
/// onto the source layer's own pixel grid, growing the layer where they now
/// extend past it. A mask without its own placement grows with it, revealing
/// the new area. Returns the merged asset, transform, and mask.
fn floating_merge(
    pixels: &Bitmap,
    transform: &LayerTransform,
    source: &ImageLayer,
) -> Option<(ImportedImage, LayerTransform, Option<LayerMask>)> {
    let source_image = source.asset.as_ref()?.image.as_ref();
    let width = source_image.width;
    let height = source_image.height;
    let to_document = crate::raster::pixel_to_document(&source.transform, width as f64, height as f64);
    let to_pixels = to_document.inverted();
    let pw = pixels.width as f64;
    let ph = pixels.height as f64;
    // Where the floating pixels land, in the source's pixel grid.
    let floating_bounds = CGRect::new(0.0, 0.0, pw, ph)
        .transformed(&crate::raster::pixel_to_document(transform, pw, ph))
        .transformed(&to_pixels);
    let original = CGRect::new(0.0, 0.0, width as f64, height as f64);
    let extent = original.union(&floating_bounds).integral();
    if extent.width() < 1.0
        || extent.height() < 1.0
        || extent.width() > 30_000.0
        || extent.height() > 30_000.0
        || extent.width() * extent.height() > 100_000_000.0
    {
        return None;
    }
    let (min_x, min_y) = (extent.min_x(), extent.min_y());
    let placed = original.offset_by(-min_x, -min_y);
    let mut canvas = Bitmap::new(extent.width() as usize, extent.height() as usize);
    crate::raster::draw_over(&mut canvas, source_image, placed, LayerSampling::High);
    // The floating pixels: grid → document → the grown source grid.
    let mapping = CGAffineTransform::translation(-min_x, -min_y)
        .concat(&to_pixels)
        .concat(&crate::raster::pixel_to_document(transform, pw, ph));
    crate::raster::draw_affine(&mut canvas, pixels, &mapping, transform.sampling, None);
    let asset = ImportedImage::new(canvas, source.name.clone());
    let mut merged = source.transform;
    merged.size = CGSize::new(
        extent.width() * source.transform.size.width / width as f64,
        extent.height() * source.transform.size.height / height as f64,
    );
    let center = CGPoint::new(extent.mid_x(), extent.mid_y()).applying(&to_document);
    merged.origin = CGPoint::new(center.x - merged.size.width / 2.0, center.y - merged.size.height / 2.0);
    let mut mask = source.mask.clone();
    if let Some(current) = source.mask.as_ref() {
        if current.placement.is_none() && extent != original {
            // Reveal the grown area: white behind the old mask.
            let mut grown = Bitmap::new(extent.width() as usize, extent.height() as usize);
            for chunk in grown.data.chunks_exact_mut(4) {
                chunk[0] = 255;
                chunk[1] = 255;
                chunk[2] = 255;
                chunk[3] = 255;
            }
            crate::raster::draw_over(
                &mut grown,
                current.asset.image.as_ref(),
                placed,
                LayerSampling::High,
            );
            mask = Some(current.replacing(ImportedImage::new(grown, "Layer Mask")));
        }
    }
    Some((asset, merged, mask))
}

/// `LiveLayerMask.adoptClipping` — after a move, a layer rejoins the clipping
/// stack that now sits right below it in the same group.
fn adopt_clipping(id: Uuid, layers: &mut [ImageLayer]) {
    let position = match layers.iter().position(|l| l.id == id) {
        Some(position) if !layers[position].is_group => position,
        _ => return,
    };
    let parent = layers[position].parent_id;
    let siblings: Vec<&ImageLayer> = layers.iter().filter(|l| l.parent_id == parent).collect();
    let index = match siblings.iter().position(|l| l.id == id) {
        Some(index) if index > 0 && index + 1 < siblings.len() => index,
        _ => return,
    };
    let source = match siblings[index + 1].mask_source_id {
        Some(source) if source != id => source,
        _ => return,
    };
    let below = siblings[index - 1];
    if below.id == source || below.mask_source_id == Some(source) {
        layers[position].mask_source_id = Some(source);
    }
}

/// `LiveLayerMask.releaseDetachedClipping` — clipping links whose base no
/// longer sits directly below them release.
fn release_detached_clipping(layers: &mut Vec<ImageLayer>) {
    let mut groups: std::collections::HashMap<Option<Uuid>, Vec<Uuid>> =
        std::collections::HashMap::new();
    for layer in layers.iter() {
        groups.entry(layer.parent_id).or_default().push(layer.id);
    }
    let by_id: std::collections::HashMap<Uuid, &ImageLayer> =
        layers.iter().map(|l| (l.id, l)).collect();
    let mut release: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    for stack in groups.values() {
        let mut base: Option<Uuid> = None;
        for id in stack {
            let layer = by_id.get(id).expect("own list");
            if let Some(source) = layer.mask_source_id {
                if Some(source) != base {
                    release.insert(*id);
                    base = Some(*id);
                }
            } else {
                base = if layer.is_group { None } else { Some(*id) };
            }
        }
    }
    for layer in layers.iter_mut() {
        if release.contains(&layer.id) {
            layer.mask_source_id = None;
        }
    }
}

/// `ImageImporter.decode`: an image file into a premultiplied canvas bitmap
/// plus its thumbnail.
fn decode_image(path: &std::path::Path) -> Option<(Bitmap, Bitmap)> {
    let image = image::open(path).ok()?;
    let rgba = image.to_rgba8();
    let (width, height) = (rgba.width() as usize, rgba.height() as usize);
    let mut bitmap = Bitmap::new(width, height);
    bitmap.data = rgba.into_raw();
    for pixel in bitmap.data.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        for channel in 0..3 {
            pixel[channel] = ((pixel[channel] as u32 * alpha + 127) / 255) as u8;
        }
    }
    let thumbnail = crate::raster::thumbnail_of(&bitmap);
    Some((bitmap, thumbnail))
}

/// A fresh pseudo-random seed for noise and grain (`UInt32.random`).
fn rand_seed() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u32)
        .unwrap_or(0)
        ^ 0x9e3779b9
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> EditorSession {
        let mut s = EditorSession::new();
        s.create_document(100, 100, true);
        s
    }

    fn red_image(size: usize) -> Bitmap {
        let mut image = Bitmap::new(size, size);
        for i in (0..image.data.len()).step_by(4) {
            image.data[i] = 255;
            image.data[i + 3] = 255;
        }
        image
    }

    #[test]
    fn new_canvas_starts_with_layer_1() {
        let s = session();
        assert_eq!(s.document.as_ref().unwrap().layers[0].name, "Layer 1");
        assert_eq!(
            s.active_layer_id,
            Some(s.document.as_ref().unwrap().layers[0].id)
        );
        assert!(s.is_modified());
    }

    #[test]
    fn blank_layers_number_sequentially() {
        let mut s = session();
        s.add_blank_layer();
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 2);
        assert_eq!(s.document.as_ref().unwrap().layers[1].name, "Layer 2");
    }

    #[test]
    fn undo_reverts_layer_add() {
        let mut s = session();
        s.add_blank_layer();
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 2);
        s.undo();
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 1);
        s.redo();
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 2);
    }

    #[test]
    fn delete_layer_removes_descendants() {
        let mut s = session();
        let group_id = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].is_group = true;
        let mut child = ImageLayer::blank("Child", CGSize::new(100.0, 100.0));
        child.parent_id = Some(group_id);
        let child_id = child.id;
        s.document.as_mut().unwrap().layers.push(child);
        s.select_layer(Some(child_id));
        s.delete_layer(child_id);
        assert_eq!(s.document.as_ref().unwrap().layers.len(), 1);
        s.select_layer(Some(group_id));
        s.delete_layer(group_id);
        assert!(s.document.as_ref().unwrap().layers.is_empty());
    }

    #[test]
    fn deleting_the_active_layer_selects_a_neighbor() {
        // LayerTests.swift:27: the selection lands on layers[min(index, count-1)].
        let mut s = session();
        s.add_blank_layer();
        s.add_blank_layer();
        let ids: Vec<Uuid> = s
            .document
            .as_ref()
            .unwrap()
            .layers
            .iter()
            .map(|l| l.id)
            .collect();
        s.select_layer(Some(ids[1]));
        s.delete_layer(ids[1]);
        assert_eq!(s.active_layer_id, Some(ids[2]));
        // Deleting the last layer moves the selection up to the new last one.
        s.select_layer(Some(ids[2]));
        s.delete_layer(ids[2]);
        assert_eq!(
            s.active_layer_id,
            s.document.as_ref().unwrap().layers.last().map(|l| l.id)
        );
    }

    #[test]
    fn rename_and_visibility_are_undoable() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.rename_layer(id, "Base");
        s.toggle_layer_visibility(id);
        assert_eq!(s.document.as_ref().unwrap().layers[0].name, "Base");
        assert!(!s.document.as_ref().unwrap().layers[0].is_visible);
        s.undo();
        s.undo();
        assert_eq!(s.document.as_ref().unwrap().layers[0].name, "Layer 1");
        assert!(s.document.as_ref().unwrap().layers[0].is_visible);
    }

    #[test]
    fn no_op_edit_preserves_history() {
        let mut s = session();
        let undo_count = s.history.undo_count();
        let id = s.active_layer_id.unwrap();
        s.rename_layer(id, "");
        assert_eq!(s.history.undo_count(), undo_count);
    }

    #[test]
    fn blend_mode_cycles_through_menu_order() {
        let mut s = session();
        s.cycle_blend_mode(true);
        assert_eq!(s.active_layer().unwrap().blend_mode, LayerBlendMode::Darken);
        s.cycle_blend_mode(false);
        assert_eq!(s.active_layer().unwrap().blend_mode, LayerBlendMode::Normal);
        s.cycle_blend_mode(false);
        assert_eq!(
            s.active_layer().unwrap().blend_mode,
            LayerBlendMode::Luminosity
        );
    }

    #[test]
    fn opacity_edit_folds_into_one_step() {
        let mut s = session();
        let before = s.history.undo_count();
        s.begin_opacity_edit();
        s.set_layer_opacity(0.3);
        s.set_layer_opacity(0.7);
        s.finish_opacity_edit();
        assert!((s.active_layer().unwrap().opacity - 0.7).abs() < 1e-9);
        assert_eq!(s.history.undo_count(), before + 1);
        assert_eq!(s.history.undo_name(), "Layer Opacity");
    }

    #[test]
    fn marquee_selects_and_composes() {
        let mut s = session();
        s.tool = NavigationTool::Marquee;
        s.begin_lasso(CGPoint::new(10.0, 10.0), SelectionMode::Replace);
        s.drag_marquee(CGPoint::new(40.0, 30.0), false, false);
        s.finish_lasso();
        let sel = s.selection().unwrap();
        assert_eq!(sel.path.bounds(), Some(CGRect::new(10.0, 10.0, 30.0, 20.0)));
        s.begin_lasso(CGPoint::new(20.0, 20.0), SelectionMode::Add);
        s.drag_marquee(CGPoint::new(50.0, 40.0), false, false);
        s.finish_lasso();
        assert_eq!(
            s.selection().unwrap().path.bounds(),
            Some(CGRect::new(10.0, 10.0, 40.0, 30.0))
        );
        s.deselect();
        assert!(s.selection().is_none());
        s.undo();
        assert!(s.selection().is_some());
    }

    #[test]
    fn lasso_click_enclosing_nothing_deselects() {
        let mut s = session();
        s.select_all();
        s.tool = NavigationTool::Lasso;
        s.begin_lasso(CGPoint::new(50.0, 50.0), SelectionMode::Replace);
        s.finish_lasso();
        assert!(s.selection().is_none());
    }

    #[test]
    fn expand_contract_and_feather() {
        let mut s = session();
        s.tool = NavigationTool::Marquee;
        s.begin_lasso(CGPoint::new(20.0, 20.0), SelectionMode::Replace);
        s.drag_marquee(CGPoint::new(40.0, 40.0), false, false);
        s.finish_lasso();
        s.expand_selection(5);
        assert_eq!(
            s.selection().unwrap().path.bounds(),
            Some(CGRect::new(15.0, 15.0, 30.0, 30.0))
        );
        s.contract_selection(5);
        assert_eq!(
            s.selection().unwrap().path.bounds(),
            Some(CGRect::new(20.0, 20.0, 20.0, 20.0))
        );
        s.feather_selection(4);
        assert!((s.selection().unwrap().feather - 4.0).abs() < 1e-9);
        s.feather_selection(3);
        let expected = (16.0f64 + 9.0).sqrt();
        assert!((s.selection().unwrap().feather - expected).abs() < 1e-9);
    }

    #[test]
    fn invert_selection() {
        let mut s = session();
        s.tool = NavigationTool::Marquee;
        s.begin_lasso(CGPoint::new(10.0, 10.0), SelectionMode::Replace);
        s.drag_marquee(CGPoint::new(20.0, 20.0), false, false);
        s.finish_lasso();
        s.invert_selection();
        let sel = s.selection().unwrap();
        assert!(!sel.path.contains(CGPoint::new(15.0, 15.0)));
        assert!(sel.path.contains(CGPoint::new(50.0, 50.0)));
    }

    #[test]
    fn magic_wand_selects_uniform_region() {
        let mut s = session();
        let mut image = red_image(100);
        for y in 0..50 {
            for x in 0..50 {
                image.set_pixel(x, y, [0, 255, 0, 255]);
            }
        }
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(image, "L"));
        s.magic_wand(CGPoint::new(10.0, 10.0), SelectionMode::Replace);
        let sel = s.selection().unwrap();
        assert!(sel.path.contains(CGPoint::new(10.0, 10.0)));
        assert!(!sel.path.contains(CGPoint::new(75.0, 75.0)));
    }

    #[test]
    fn add_and_delete_mask() {
        let mut s = session();
        s.add_layer_mask(true);
        assert!(s.active_layer().unwrap().mask.is_some());
        assert!(s.is_mask_selected);
        s.toggle_layer_mask();
        assert!(!s.active_layer().unwrap().mask.as_ref().unwrap().is_enabled);
        s.delete_layer_mask();
        assert!(s.active_layer().unwrap().mask.is_none());
    }

    #[test]
    fn transform_moves_layer_as_one_edit() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(50, 50), "L"));
        s.begin_transform();
        assert!(s.transform_edit.is_some());
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.origin = CGPoint::new(10.0, 5.0);
        }
        s.commit_transform();
        let layer = s.document.as_ref().unwrap().layer(id).unwrap();
        assert_eq!(layer.transform.origin, CGPoint::new(10.0, 5.0));
        assert_eq!(s.history.undo_name(), "Transform Layer");
    }

    #[test]
    fn mask_only_transform_moves_the_placement() {
        // An unlinked mask dragged on its own takes the placement; the layer
        // stays put and the undo step is named for the mask.
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(50, 50), "L"));
        {
            let doc = s.document.as_mut().unwrap();
            let layer = doc.layer_mut(id).unwrap();
            let mut mask = LayerMask::solid(true);
            mask.is_linked = false;
            layer.mask = Some(mask);
        }
        s.is_mask_selected = true;
        s.begin_transform();
        assert!(s.transform_edit.is_some());
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.origin = CGPoint::new(10.0, 5.0);
        }
        s.commit_transform();
        let layer = s.document.as_ref().unwrap().layer(id).unwrap();
        assert_eq!(layer.transform.origin, CGPoint::ZERO);
        assert_eq!(
            layer.mask.as_ref().unwrap().placement.map(|p| p.origin),
            Some(CGPoint::new(10.0, 5.0))
        );
        assert_eq!(s.history.undo_name(), "Transform Layer Mask");
    }

    #[test]
    fn magic_wand_on_a_blank_layer_selects_the_canvas() {
        // selectionSample reads a blank layer as transparent; the wand then
        // matches the empty canvas (MagicWandTests.swift:101).
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset = None;
        s.magic_wand(CGPoint::new(50.0, 50.0), SelectionMode::Replace);
        let sel = s.selection().expect("blank layer still selects");
        let bounds = sel.path.bounds().expect("nonempty path");
        assert!(bounds.width() >= 99.0, "{bounds:?}");
        assert!(bounds.height() >= 99.0, "{bounds:?}");
    }

    fn rectangular_selection(s: &mut EditorSession, x0: f64, y0: f64, x1: f64, y1: f64) {
        s.tool = NavigationTool::Marquee;
        s.begin_lasso(CGPoint::new(x0, y0), SelectionMode::Replace);
        s.drag_marquee(CGPoint::new(x1, y1), false, false);
        s.finish_lasso();
    }

    #[test]
    fn fill_selection_fills_only_the_selection() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(100, 100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.fill_selection(false);
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert_eq!(image.pixel(20, 20)[3], 255, "inside fills");
        assert_eq!(image.pixel(20, 20)[0], 0, "default foreground is black");
        assert_eq!(image.pixel(60, 60)[3], 0, "outside stays untouched");
        assert_eq!(s.history.undo_name(), "Fill");
    }

    #[test]
    fn clear_selected_pixels_clears_only_the_selection() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.clear_selected_pixels();
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert_eq!(image.pixel(20, 20)[3], 0);
        assert_eq!(image.pixel(60, 60), [255, 0, 0, 255]);
        assert_eq!(s.history.undo_name(), "Clear");
    }

    #[test]
    fn invert_pixels_inverts_inside_the_selection() {
        let mut s = session();
        let mut image = red_image(100);
        for y in 0..100 {
            for x in 0..100 {
                // Black left half, white right half.
                let v = if x < 50 { 0 } else { 255 };
                image.set_pixel(x, y, [v, v, v, 255]);
            }
        }
        s.document.as_mut().unwrap().layers[0].asset = Some(ImportedImage::new(image, "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.invert_pixels();
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert!(image.pixel(20, 20)[0] > 200, "black inverts to white in place");
        assert_eq!(image.pixel(75, 75)[0], 255, "unselected white stays");
        assert_eq!(image.pixel(5, 75)[0], 0, "unselected black stays");
        assert_eq!(s.history.undo_name(), "Invert");
    }

    #[test]
    fn mask_from_selection_knocks_out_the_selection() {
        // Add White Mask with a selection: white everywhere but the selected
        // pixels (LayerMask.addMask(revealing:) fills the selection with the
        // opposite gray).
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.add_mask_from_selection(true);
        assert!(s.selection().is_none(), "the selection becomes the mask");
        assert!(s.is_mask_selected);
        let mask = s.active_layer().unwrap().mask.as_ref().unwrap();
        let image = mask.asset.image.as_ref();
        assert_eq!((image.width, image.height), (100, 100));
        assert!(image.pixel(20, 20)[0] < 55, "selected: knocked out");
        assert!(image.pixel(60, 60)[0] > 200, "unselected: revealed");
        assert_eq!(s.history.undo_name(), "Add Mask from Selection");
    }

    #[test]
    fn add_adjustment_inserts_a_layer_above_the_active_one() {
        let mut s = session();
        s.add_adjustment(crate::adjustment::AdjustmentKind::Exposure);
        let document = s.document.as_ref().unwrap();
        assert_eq!(document.layers.len(), 2);
        let top = document.layers.last().unwrap();
        assert!(top.adjustment.is_some());
        assert!(top.asset.is_none());
        assert_eq!(s.active_layer_id, Some(top.id));
        assert_eq!(s.history.undo_name(), "New Exposure Adjustment");
    }

    #[test]
    fn duplicating_a_folder_copies_descendants_and_remaps_links() {
        // duplicateActiveLayer: a folder copies with its contents, the copy is
        // named " copy", and children land under the copied folder.
        let mut s = session();
        let folder_id = s.document.as_ref().unwrap().layers[0].id;
        {
            let doc = s.document.as_mut().unwrap();
            doc.layers[0].is_group = true;
            let mut child = ImageLayer::blank("Child", CGSize::new(100.0, 100.0));
            child.parent_id = Some(folder_id);
            doc.layers.push(child);
        }
        let child_id = s.document.as_ref().unwrap().layers[1].id;
        s.select_layer(Some(folder_id));
        s.duplicate_active_layer();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 4);
        // insert(contentsOf:at:) lands the copies as a block: folder,
        // folder copy, child copy, original child.
        let copy = &doc.layers[1];
        assert_eq!(copy.name, "Layer 1 copy");
        let copied_child = &doc.layers[2];
        assert_eq!(copied_child.name, "Child");
        assert_eq!(copied_child.parent_id, Some(copy.id));
        assert_eq!(doc.layers[3].id, child_id, "the original child stays");
        assert_eq!(s.history.undo_name(), "Duplicate Layer");
    }

    #[test]
    fn copy_paste_round_trips_pixels_in_place() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.copy_selection();
        let clipboard = s.pixel_clipboard.as_ref().unwrap();
        assert_eq!((clipboard.image.width, clipboard.image.height), (20, 20));
        assert_eq!(clipboard.origin, CGPoint::new(10.0, 10.0));
        // Pasting adds a layer above with the pixels where they came from.
        s.paste_pixels();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 2);
        let pasted = doc.layers.last().unwrap();
        assert_eq!(pasted.transform.origin, CGPoint::new(10.0, 10.0));
        assert_eq!(pasted.asset.as_ref().unwrap().image.pixel(10, 10), [255, 0, 0, 255]);
        assert!(s.selection().is_none(), "pasting drops the selection");
        assert_eq!(s.history.undo_name(), "Paste");
    }

    #[test]
    fn cut_selection_copies_then_clears() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.cut_selection();
        let image = s
            .active_layer()
            .unwrap()
            .asset
            .as_ref()
            .unwrap()
            .image
            .as_ref()
            .clone();
        assert_eq!(image.pixel(20, 20)[3], 0, "cut pixels leave the layer");
        assert_eq!(image.pixel(60, 60), [255, 0, 0, 255], "the rest stays");
        assert!(s.pixel_clipboard.is_some(), "cut pixels land on the clipboard");
    }

    #[test]
    fn layer_via_copy_lifts_the_selection_into_a_layer() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        let active_before = s.active_layer_id;
        s.layer_via_copy();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 2);
        let copied = doc.layers.last().unwrap();
        assert_ne!(Some(copied.id), active_before);
        assert_eq!(copied.transform.origin, CGPoint::new(10.0, 10.0));
        assert_eq!(copied.asset.as_ref().unwrap().image.pixel(5, 5), [255, 0, 0, 255]);
        assert_eq!(s.history.undo_name(), "Layer via Copy");
    }

    #[test]
    fn floating_transform_lifts_pixels_onto_a_temporary_layer() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.begin_selection_transform();
        assert!(s.transform_edit.as_ref().unwrap().floating.is_some());
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 2, "the floating layer joins the stack");
        let floating = &doc.layers[1];
        assert_eq!(floating.name, "Floating Selection");
        assert_eq!(floating.transform.origin, CGPoint::new(10.0, 10.0));
        assert_eq!(floating.asset.as_ref().unwrap().image.pixel(5, 5), [255, 0, 0, 255]);
        assert_eq!(s.active_layer_id, Some(floating.id));
        // The source keeps the rest of its pixels but the selection is empty.
        let source = &doc.layers[0];
        assert_eq!(source.asset.as_ref().unwrap().image.pixel(20, 20)[3], 0);
        assert_eq!(source.asset.as_ref().unwrap().image.pixel(60, 60), [255, 0, 0, 255]);
    }

    #[test]
    fn committing_an_untouched_floating_transform_restores_the_document() {
        // No-op merge: restore exactly, so soft selection edges never pick up
        // a seam.
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.begin_selection_transform();
        s.commit_transform();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        let layer = &doc.layers[0];
        assert_eq!(layer.asset.as_ref().unwrap().image.pixel(20, 20), [255, 0, 0, 255]);
        assert_eq!(s.active_layer_id, Some(layer.id));
    }

    #[test]
    fn merging_a_moved_floating_selection_moves_pixels_and_selection() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.begin_selection_transform();
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.origin = CGPoint::new(20.0, 15.0);
        }
        s.commit_transform();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1, "the floating layer merges away");
        let image = doc.layers[0].asset.as_ref().unwrap().image.as_ref().clone();
        assert_eq!(image.pixel(12, 12)[3], 0, "the vacated area stays clear");
        assert_eq!(image.pixel(25, 20), [255, 0, 0, 255], "the pixels land moved");
        assert_eq!(image.pixel(60, 60), [255, 0, 0, 255], "the rest stays");
        let selection = doc.selection.as_ref().unwrap();
        let bounds = selection.path.bounds().unwrap();
        assert!((bounds.min_x() - 20.0).abs() <= 1.0, "{bounds:?}");
        assert!((bounds.min_y() - 15.0).abs() <= 1.0, "{bounds:?}");
        assert_eq!(s.active_layer_id, Some(doc.layers[0].id));
        // One undo step puts the whole thing back.
        s.undo();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].asset.as_ref().unwrap().image.pixel(20, 20), [255, 0, 0, 255]);
    }

    #[test]
    fn cancelling_a_floating_transform_restores_everything() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        rectangular_selection(&mut s, 10.0, 10.0, 30.0, 30.0);
        s.begin_selection_transform();
        if let Some(edit) = s.transform_edit.as_mut() {
            edit.draft.origin = CGPoint::new(50.0, 50.0);
        }
        s.cancel_transform();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].asset.as_ref().unwrap().image.pixel(20, 20), [255, 0, 0, 255]);
        assert!(doc.selection.is_some(), "the selection comes back");
        assert_eq!(s.active_layer_id, Some(doc.layers[0].id));
    }

    fn blank_layer(s: &mut EditorSession, name: &str) -> Uuid {
        let mut layer = ImageLayer::blank(name, CGSize::new(100.0, 100.0));
        layer.id = uuid::Uuid::new_v4();
        let id = layer.id;
        s.document.as_mut().unwrap().layers.push(layer);
        id
    }

    #[test]
    fn group_selected_layers_wraps_the_selection() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = blank_layer(&mut s, "Second");
        let third = blank_layer(&mut s, "Third");
        s.select_layers(&[first, second], Some(second));
        s.group_selected_layers();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 4);
        let folder = doc.layers.first().unwrap().clone();
        assert!(folder.is_group);
        assert_eq!(folder.name, "Folder 1");
        // The two selected layers are the folder's children.
        let children: Vec<&ImageLayer> = doc.layers.iter().filter(|l| l.parent_id == Some(folder.id)).collect();
        assert_eq!(children.len(), 2);
        let ids: Vec<Uuid> = children.iter().map(|l| l.id).collect();
        assert!(ids.contains(&first) && ids.contains(&second));
        assert!(!ids.contains(&third));
        assert_eq!(s.active_layer_id, Some(folder.id));
        assert_eq!(s.history.undo_name(), "Group Layers");
        s.undo();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 3);
        assert!(doc.layers.iter().all(|l| l.parent_id.is_none()));
    }

    #[test]
    fn grouping_a_folder_carries_its_subtree() {
        // Selected descendants must not be pulled out of a selected folder:
        // the folder is a root, its child rides along.
        let mut s = session();
        let folder_id = s.document.as_ref().unwrap().layers[0].id;
        {
            let doc = s.document.as_mut().unwrap();
            doc.layers[0].is_group = true;
            let mut child = ImageLayer::blank("Child", CGSize::new(100.0, 100.0));
            child.parent_id = Some(folder_id);
            doc.layers.push(child);
        }
        let child_id = s.document.as_ref().unwrap().layers[1].id;
        let sibling = blank_layer(&mut s, "Sibling");
        s.select_layers(&[folder_id, child_id], Some(folder_id));
        s.group_selected_layers();
        let doc = s.document.as_ref().unwrap();
        // Only the folder lands in the new group; the child stays with it.
        let new_folder = doc.layers.first().unwrap();
        assert_eq!(new_folder.name, "Folder 1");
        let folder_child: Vec<&ImageLayer> = doc
            .layers
            .iter()
            .filter(|l| l.parent_id == Some(new_folder.id))
            .collect();
        assert_eq!(folder_child.len(), 1);
        assert_eq!(folder_child[0].id, folder_id);
        let _ = sibling;
    }

    #[test]
    fn collapse_hides_rows_but_layers_rows_show_folders() {
        let mut s = session();
        let folder_id = s.document.as_ref().unwrap().layers[0].id;
        {
            let doc = s.document.as_mut().unwrap();
            doc.layers[0].is_group = true;
            let mut child = ImageLayer::blank("Child", CGSize::new(100.0, 100.0));
            child.parent_id = Some(folder_id);
            doc.layers.push(child);
        }
        let rows = s.layer_rows();
        assert_eq!(rows.len(), 2, "folder then child");
        assert_eq!((rows[0].depth, rows[1].depth), (0, 1));
        s.toggle_group_expansion(folder_id);
        let rows = s.layer_rows();
        assert_eq!(rows.len(), 1, "the child row folds away");
        // Toggling back restores it.
        s.toggle_group_expansion(folder_id);
        assert_eq!(s.layer_rows().len(), 2);
    }

    #[test]
    fn place_layer_moves_into_groups_and_refuses_cycles() {
        let mut s = session();
        let layer_id = s.document.as_ref().unwrap().layers[0].id;
        s.add_group();
        let folder_id = s.active_layer_id.unwrap();
        // A folder can never be placed inside its own subtree.
        assert!(!s.can_place_layer(folder_id, Some(folder_id)));
        assert!(s.place_layer(layer_id, Some(folder_id), None, false));
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layer(layer_id).unwrap().parent_id, Some(folder_id));
        assert_eq!(s.history.undo_name(), "Move Layer");
        // Out again: above the group, at the group's parent (the root).
        s.select_layer(Some(layer_id));
        s.move_active_layer_out_of_group();
        assert_eq!(
            s.document
                .as_ref()
                .unwrap()
                .layer(layer_id)
                .unwrap()
                .parent_id,
            None
        );
    }

    #[test]
    fn reorder_layers_moves_in_top_first_panel_order() {
        // Document order is bottom-to-top: [A, B]. The panel shows [B, A].
        let mut s = session();
        let a = s.document.as_ref().unwrap().layers[0].id;
        let b = blank_layer(&mut s, "B");
        // Panel order: [B, A]. Moving panel row 0 (B) to the panel end drops B
        // to the bottom of the canvas — the first slot in storage order.
        s.reorder_layers(&[0], 2);
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.first().unwrap().id, b, "B lands at the bottom");
        assert_eq!(doc.layers.last().unwrap().id, a, "A rises to the top");
    }

    #[test]
    fn visibility_swipe_batches_into_one_undo_step() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = blank_layer(&mut s, "B");
        let before = s.history.undo_count();
        s.begin_visibility_swipe(first).unwrap();
        s.set_visibility_in_swipe(second, false);
        s.end_visibility_swipe();
        let doc = s.document.as_ref().unwrap();
        assert!(!doc.layer(first).unwrap().is_visible);
        assert!(!doc.layer(second).unwrap().is_visible);
        assert_eq!(s.history.undo_count(), before + 1, "one step for the swipe");
        s.undo();
        let doc = s.document.as_ref().unwrap();
        assert!(doc.layer(first).unwrap().is_visible);
        assert!(doc.layer(second).unwrap().is_visible);
    }

    #[test]
    fn delete_selected_layers_takes_every_selection_in_one_step() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = blank_layer(&mut s, "B");
        let third = blank_layer(&mut s, "C");
        s.select_layers(&[first, second], Some(first));
        s.delete_selected_layers();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].id, third);
        assert_eq!(s.active_layer_id, Some(third));
        assert_eq!(s.history.undo_name(), "Delete Layers");
    }

    #[test]
    fn importing_images_creates_a_canvas_then_centers_layers() {
        let mut s = EditorSession::new();
        let dir = std::env::temp_dir();
        let first = dir.join(format!("compositor-import-{}-a.png", std::process::id()));
        let image = image::RgbaImage::from_pixel(20, 10, image::Rgba([200, 40, 40, 255]));
        image.save_with_format(&first, image::ImageFormat::Png).unwrap();
        let failures = s.import_images(&[first.clone()], None);
        assert!(failures.is_empty(), "{failures:?}");
        let doc = s.document.as_ref().expect("the first image makes the canvas");
        assert_eq!((doc.width, doc.height), (20, 10));
        assert_eq!(doc.layers.len(), 1);
        // A second image centers on the canvas.
        let second = dir.join(format!("compositor-import-{}-b.png", std::process::id()));
        let image = image::RgbaImage::from_pixel(10, 10, image::Rgba([40, 40, 200, 255]));
        image.save_with_format(&second, image::ImageFormat::Png).unwrap();
        let failures = s.import_images(&[second.clone()], None);
        assert!(failures.is_empty(), "{failures:?}");
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 2);
        let pasted = doc.layers.last().unwrap();
        assert_eq!(pasted.transform.origin, CGPoint::new(5.0, 0.0));
        assert_eq!(s.history.undo_name(), "Import Images");
        // Unreadable paths report failures.
        let failures = s.import_images(&[dir.join("compositor-missing-file.png")], None);
        assert_eq!(failures.len(), 1);
        let _ = std::fs::remove_file(first);
        let _ = std::fs::remove_file(second);
    }

    #[test]
    fn import_size_budget_rejects_oversized_bitmaps() {
        let mut s = session();
        // Over the per-side cap.
        assert!(!s.import_size_allowed(30_001, 1));
        // Over the 100-megapixel document budget once content exists.
        assert!(s.import_size_allowed(5_000, 5_000), "25 MP fits an empty canvas");
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(9_500), "L")); // 90.25 MP
        assert!(!s.import_size_allowed(5_000, 5_000), "90.25 + 25 exceeds 100 MP");
    }

    #[test]
    fn applying_text_creates_a_live_text_layer() {
        let mut s = session();
        let point = CGPoint::new(20.0, 15.0);
        s.begin_text(point, true);
        let mut draft = s.text_draft.clone().unwrap();
        draft.style.content = "Hello".to_string();
        assert!(s.apply_text(draft));
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 2);
        let layer = doc.layers.last().unwrap();
        assert_eq!(layer.name, "Hello");
        // The layer is live text: its pixels carry the rasterized glyphs.
        let live = layer.live_text().expect("live text");
        assert_eq!(live.style.content, "Hello");
        assert!(live.image.width >= 16);
        assert!(
            live.image.data.chunks_exact(4).any(|px| px[3] > 0),
            "glyphs are rasterized"
        );
        assert_eq!(s.active_layer_id, Some(layer.id));
        assert_eq!(s.history.undo_name(), "New Text Layer");
        // The style becomes the default for the next text layer.
        assert_eq!(s.text_defaults.content, "Hello");
    }

    #[test]
    fn applying_empty_new_text_is_a_quiet_noop() {
        let mut s = session();
        let before = s.document.as_ref().unwrap().layers.len();
        s.begin_text(CGPoint::new(20.0, 15.0), true);
        let draft = s.text_draft.clone().unwrap();
        assert!(draft.style.content.is_empty());
        assert!(s.apply_text(draft));
        assert_eq!(s.document.as_ref().unwrap().layers.len(), before);
        assert!(s.text_draft.is_none());
    }

    #[test]
    fn editing_updates_the_live_text_layer() {
        let mut s = session();
        let point = CGPoint::new(20.0, 15.0);
        s.begin_text(point, true);
        let mut draft = s.text_draft.clone().unwrap();
        draft.style.content = "First".to_string();
        s.apply_text(draft);
        let layer_id = s.document.as_ref().unwrap().layers.last().unwrap().id;
        // A click inside the layer edits it rather than making a new one.
        s.begin_text(CGPoint::new(25.0, 20.0), false);
        let draft = s.text_draft.clone().expect("draft targets the layer");
        assert_eq!(draft.layer_id, Some(layer_id));
        let mut edited = draft;
        edited.style.content = "Second".to_string();
        assert!(s.apply_text(edited));
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 2, "no extra layer");
        let layer = doc.layer(layer_id).unwrap();
        assert_eq!(layer.live_text().unwrap().style.content, "Second");
        assert_eq!(s.history.undo_name(), "Edit Text");
    }

    #[test]
    fn recolor_text_keeps_it_editable() {
        let mut s = session();
        let point = CGPoint::new(20.0, 15.0);
        s.begin_text(point, true);
        let mut draft = s.text_draft.clone().unwrap();
        draft.style.content = "Tint".to_string();
        s.apply_text(draft);
        let layer_id = s.document.as_ref().unwrap().layers.last().unwrap().id;
        assert!(s.recolor_text(layer_id, PaletteColor::new(1.0, 0.0, 0.0)));
        let layer = s.document.as_ref().unwrap().layer(layer_id).unwrap();
        let live = layer.live_text().expect("still live text");
        assert_eq!((live.style.red, live.style.green, live.style.blue), (1.0, 0.0, 0.0));
        assert_eq!(s.history.undo_name(), "Fill Text");
    }

    #[test]
    fn moving_snaps_to_the_canvas_edge() {
        // A layer at (50,50,10,10): dragging it so its right/bottom edge comes
        // within tolerance of the canvas's right/bottom edge snaps it to
        // x = 90, y = 90.
        let mut s = session();
        let id = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(10), "L"));
        s.document.as_mut().unwrap().layers[0].transform =
            LayerTransform::new(CGPoint::new(50.0, 50.0), CGSize::new(10.0, 10.0));
        s.select_layer(Some(id));
        s.begin_transform();
        let mut draft = s.transform_edit.as_ref().unwrap().draft;
        draft.origin = CGPoint::new(51.0, 51.0);
        let snapped = s.snapped_move(draft, &[id], 10.0);
        // The draft's right edge lands at 101, one pixel shy of the canvas —
        // within the 10 px tolerance.
        assert!((snapped.origin.x - 50.0).abs() <= 11.0, "{snapped:?}");
        assert!((snapped.origin.y - 50.0).abs() <= 11.0, "{snapped:?}");
        let _ = id;
        // An exact positioning: max at 101 snaps to 100 → origin 90.
        draft.origin = CGPoint::new(91.0, 91.0);
        let snapped = s.snapped_move(draft, &[id], 10.0);
        assert!((snapped.origin.x - 90.0).abs() <= 10.0, "{snapped:?}");
    }

    #[test]
    fn snapping_off_disables_move_snapping() {
        let mut s = session();
        s.snapping_enabled = false;
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(10), "L"));
        s.document.as_mut().unwrap().layers[0].transform =
            LayerTransform::new(CGPoint::new(50.0, 50.0), CGSize::new(10.0, 10.0));
        s.select_layer(Some(s.document.as_ref().unwrap().layers[0].id));
        s.begin_transform();
        let mut draft = s.transform_edit.as_ref().unwrap().draft;
        draft.origin = CGPoint::new(91.0, 91.0);
        let moved = s.snapped_move(draft, &[], 10.0);
        assert!((moved.origin.x - 91.0).abs() < 1e-9, "no snap: {moved:?}");
        assert_eq!(s.snap_guides, (Vec::new(), Vec::new()));
    }

    #[test]
    fn change_text_style_edits_the_pending_draft() {
        let mut s = session();
        s.begin_text(CGPoint::new(20.0, 15.0), true);
        s.change_text_style(|style| style.font_size = 100.0);
        assert_eq!(s.text_draft.as_ref().unwrap().style.font_size, 100.0);
        assert_eq!(s.current_text_style().font_size, 100.0);
        // With no draft and no live text, edits land in the defaults.
        let mut s = session();
        s.change_text_style(|style| style.font_size = 36.0);
        assert_eq!(s.text_defaults.font_size, 36.0);
    }

    #[test]
    fn moving_the_crop_frame_snaps_both_edges() {
        // The frame's left edge comes within tolerance of the canvas's left
        // edge (x = 0): the move snaps onto it, keeping the size.
        let mut s = session();
        s.crop_rect = Some(CGRect::new(6.0, 20.0, 50.0, 40.0));
        s.begin_crop_drag(CGPoint::new(8.0, 20.0), crate::canvas::CropDragMode::Move);
        s.drag_crop(CGPoint::new(-2.0, 20.0), None, false);
        let rect = s.crop_rect.expect("crop active");
        assert_eq!(rect.min_x(), 0.0, "left edge snapped to the canvas");
        assert_eq!(rect.width(), 50.0, "the size is unchanged by a move");
    }

    #[test]
    fn resizing_snaps_only_the_dragged_edge() {
        let mut s = session();
        s.crop_rect = Some(CGRect::new(20.0, 20.0, 50.0, 40.0));
        // Drag the right-middle handle: the right edge snaps to the canvas's
        // right edge, the left edge stays put.
        s.begin_crop_drag(
            CGPoint::new(70.0, 40.0),
            crate::canvas::CropDragMode::Resize(2),
        );
        s.drag_crop(CGPoint::new(101.0, 40.0), None, false);
        let rect = s.crop_rect.expect("crop active");
        assert!((rect.max_x() - 100.0).abs() <= 1.0, "right edge snapped: {rect:?}");
        assert!((rect.min_x() - 20.0).abs() <= 1.0, "left edge untouched: {rect:?}");
    }

    #[test]
    fn a_fixed_ratio_skips_edge_snapping() {
        let mut s = session();
        s.crop_rect = Some(CGRect::new(4.0, 20.0, 50.0, 40.0));
        s.begin_crop_drag(CGPoint::new(2.0, 20.0), crate::canvas::CropDragMode::Move);
        s.drag_crop(CGPoint::new(-2.0, 20.0), Some(1.5), false);
        let rect = s.crop_rect.expect("crop active");
        assert!((rect.min_x() - 2.0).abs() < 3.0, "no snap with a ratio: {rect:?}");
    }

    #[test]
    fn snapping_off_leaves_the_frame_alone() {
        let mut s = session();
        s.snapping_enabled = false;
        s.crop_rect = Some(CGRect::new(4.0, 20.0, 50.0, 40.0));
        s.begin_crop_drag(CGPoint::new(2.0, 20.0), crate::canvas::CropDragMode::Move);
        s.drag_crop(CGPoint::new(-2.0, 20.0), None, false);
        let rect = s.crop_rect.expect("crop active");
        assert!((rect.min_x() - 2.0).abs() < 3.0, "free placement: {rect:?}");
    }

    #[test]
    fn toggle_clipping_mask_clips_to_the_lower_sibling() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = blank_layer(&mut s, "B");
        let third = blank_layer(&mut s, "C");
        // The bottom layer has nothing above it to clip to — and cannot clip.
        assert!(!s.can_toggle_clipping_mask(first));
        // Option-click the middle layer: clipped to the bottom one.
        s.toggle_clipping_mask(second);
        assert_eq!(
            s.document.as_ref().unwrap().layer(second).unwrap().mask_source_id,
            Some(first)
        );
        assert_eq!(s.history.undo_name(), "Create Clipping Mask");
        // Option-click the top layer: it shares the bottom one as its base.
        s.toggle_clipping_mask(third);
        assert_eq!(
            s.document.as_ref().unwrap().layer(third).unwrap().mask_source_id,
            Some(first)
        );
        // Releasing the first clipped child releases the whole stack above it.
        s.remove_live_mask(second);
        assert_eq!(s.document.as_ref().unwrap().layer(second).unwrap().mask_source_id, None);
        assert_eq!(s.document.as_ref().unwrap().layer(third).unwrap().mask_source_id, None);
        assert_eq!(s.history.undo_name(), "Release Clipping Mask");
    }

    #[test]
    fn releasing_a_child_also_releases_the_clipped_sibling_above_it() {
        // Releasing a child of a shared base releases the contiguous stack
        // above it sharing that base; lower siblings stay put.
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = blank_layer(&mut s, "B");
        let third = blank_layer(&mut s, "C");
        s.toggle_clipping_mask(second);
        s.toggle_clipping_mask(third);
        s.toggle_clipping_mask(second);
        assert_eq!(s.document.as_ref().unwrap().layer(second).unwrap().mask_source_id, None);
        assert_eq!(s.document.as_ref().unwrap().layer(third).unwrap().mask_source_id, None);
        assert_eq!(s.history.undo_name(), "Release Clipping Mask");
        let _ = first;
    }

    #[test]
    fn link_mask_refuses_cycles() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        let second = blank_layer(&mut s, "B");
        s.toggle_clipping_mask(second);
        // Linking the base to its own clipped child would close a cycle.
        assert!(!s.can_link_mask(second, first));
        assert!(!s.link_mask(second, first));
        assert_eq!(s.document.as_ref().unwrap().layer(first).unwrap().mask_source_id, None);
    }

    #[test]
    fn baking_carries_the_clipped_appearance_into_pixels() {
        let mut s = session();
        let base = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(100), "L"));
        let mut target_layer = ImageLayer::blank("B", CGSize::new(100.0, 100.0));
        target_layer.asset = Some(ImportedImage::new(Bitmap::new(20, 20), "B"));
        let target = target_layer.id;
        s.document.as_mut().unwrap().layers.push(target_layer);
        s.toggle_clipping_mask(target);
        // Deleting the base with bake keeps the clipped look in the pixels.
        assert!(s.delete_with_live_mask_choice(&[base], true));
        let doc = s.document.as_ref().unwrap();
        assert!(doc.layer(base).is_none(), "the base is gone");
        let kept = doc.layer(target).unwrap();
        assert_eq!(kept.mask_source_id, None, "the link is gone");
        let image = kept.asset.as_ref().unwrap().image.as_ref().clone();
        assert_eq!(image.pixel(5, 5), [255, 0, 0, 255], "the base is baked in");
    }

    #[test]
    fn deleting_without_dependencies_reports_nothing() {
        let mut s = session();
        let first = s.document.as_ref().unwrap().layers[0].id;
        assert!(!s.delete_with_live_mask_choice(&[first], true));
    }

    #[test]
    fn distorting_resamples_pixels_into_the_shape() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(40), "L"));
        s.begin_transform();
        s.begin_distort();
        assert!(s.transform_edit.as_ref().unwrap().corners.is_some());
        // Drag the top-left corner inward; an unusable shape is refused.
        s.preview_corners([CGPoint::ZERO; 4]);
        assert!(s.transform_edit.as_ref().unwrap().corners.is_some());
        let shape = [
            CGPoint::new(10.0, 0.0),
            CGPoint::new(50.0, 0.0),
            CGPoint::new(50.0, 40.0),
            CGPoint::new(10.0, 40.0),
        ];
        s.preview_corners(shape);
        s.commit_transform();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        let layer = &doc.layers[0];
        assert_eq!(layer.transform.origin, CGPoint::new(10.0, 0.0));
        assert_eq!(layer.transform.size, CGSize::new(40.0, 40.0));
        let image = layer.asset.as_ref().unwrap().image.as_ref().clone();
        assert!(image.width >= 38 && image.width <= 42, "{}", image.width);
        // One undo step restores the un-distorted layer.
        assert_eq!(s.history.undo_name(), "Distort");
        s.undo();
        let layer = s.document.as_ref().unwrap().layers[0].clone();
        assert_eq!(layer.transform.origin, CGPoint::ZERO);
        assert_eq!(layer.asset.as_ref().unwrap().image.width, 40);
    }

    #[test]
    fn distorting_a_linked_mask_warps_it_with_the_pixels() {
        let mut s = session();
        let id = s.document.as_ref().unwrap().layers[0].id;
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(red_image(40), "L"));
        {
            let doc = s.document.as_mut().unwrap();
            let layer = doc.layer_mut(id).unwrap();
            let mut mask = LayerMask::solid(true);
            mask.is_linked = true;
            mask.asset = ImportedImage::new(red_image(40), "Layer Mask");
            layer.mask = Some(mask);
        }
        s.begin_transform();
        s.begin_distort();
        let shape = [
            CGPoint::new(10.0, 0.0),
            CGPoint::new(50.0, 0.0),
            CGPoint::new(50.0, 40.0),
            CGPoint::new(10.0, 40.0),
        ];
        s.preview_corners(shape);
        s.commit_transform();
        let doc = s.document.as_ref().unwrap();
        let layer = &doc.layers[0];
        let mask = layer.mask.as_ref().unwrap();
        // The mask warped with the pixels: still linked, no placement, and it
        // picked up the pixels' new bounds.
        assert!(mask.is_linked);
        assert!(mask.placement.is_none());
        assert!(mask.asset.image.width >= 38, "{}", mask.asset.image.width);
        assert_eq!(s.history.undo_name(), "Distort");
    }

    #[test]
    fn nudge_layer_commits_immediately() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(50, 50), "L"));
        s.nudge_layer(1.0, 0.0);
        assert_eq!(
            s.document
                .as_ref()
                .unwrap()
                .layer(id)
                .unwrap()
                .transform
                .origin
                .x,
            1.0
        );
        assert!(s.transform_edit.is_none());
    }

    #[test]
    fn flip_layers_toggles_flip_x() {
        let mut s = session();
        let id = s.active_layer_id.unwrap();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(50, 50), "L"));
        s.flip_layers(true);
        let layer = s.document.as_ref().unwrap().layer(id).unwrap();
        assert!(layer.transform.flip_x);
        // The flip is about the layer's own middle: the center is unchanged.
        assert!((layer.transform.center().x - 50.0).abs() < 1e-9);
        assert!((layer.transform.center().y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn merge_down_bakes_pixels() {
        let mut s = session();
        s.add_blank_layer();
        let top_id = s.document.as_ref().unwrap().layers[1].id;
        {
            let doc = s.document.as_mut().unwrap();
            doc.layers[0].asset = Some(ImportedImage::new(red_image(50), "bottom"));
            let mut blue = Bitmap::new(50, 50);
            for i in (0..blue.data.len()).step_by(4) {
                blue.data[i + 2] = 255;
                blue.data[i + 3] = 255;
            }
            doc.layers[1].asset = Some(ImportedImage::new(blue, "top"));
        }
        s.select_layer(Some(top_id));
        s.merge_layers();
        let doc = s.document.as_ref().unwrap();
        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.layers[0].id, s.active_layer_id.unwrap());
        let out = doc.layers[0].asset.as_ref().unwrap().image.clone();
        assert_eq!(out.pixel(10, 10), [0, 0, 255, 255]);
    }

    #[test]
    fn guides_add_clear() {
        let mut s = session();
        s.add_guide(GuideAxis::Vertical, 50.0);
        assert_eq!(s.document.as_ref().unwrap().guides.len(), 1);
        s.clear_guides();
        assert!(s.document.as_ref().unwrap().guides.is_empty());
    }

    #[test]
    fn crop_shifts_content() {
        let mut s = session();
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(Bitmap::new(100, 100), "L"));
        s.crop_rect = Some(CGRect::new(25.0, 25.0, 50.0, 50.0));
        s.commit_crop();
        let doc = s.document.as_ref().unwrap();
        assert_eq!((doc.width, doc.height), (50, 50));
        assert_eq!(doc.layers[0].transform.origin, CGPoint::new(-25.0, -25.0));
    }

    #[test]
    fn canvas_and_image_size() {
        let mut s = session();
        let mut options = CanvasSizeOptions::new(150, 150);
        options.anchor = 0;
        s.resize_canvas_to(&options);
        assert_eq!(s.document.as_ref().unwrap().width, 150);
        s.resize_image_to(75, 75, Some(144.0));
        assert_eq!(s.document.as_ref().unwrap().width, 75);
        assert_eq!(s.document.as_ref().unwrap().resolution, 144.0);
        assert_eq!(s.history.undo_name(), "Image Size");
    }

    #[test]
    fn trim_reports_nothing_to_trim() {
        let mut s = session();
        // A fully transparent layer: nothing remains to trim.
        let transparent = Bitmap::new(100, 100);
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(transparent, "L"));
        assert!(!s.trim(&TrimOptions::default()));
        // Content in one corner: trim crops to it.
        let mut image = Bitmap::new(100, 100);
        for y in 0..20 {
            for x in 0..20 {
                image.set_pixel(x, y, [255, 0, 0, 255]);
            }
        }
        s.document.as_mut().unwrap().layers[0].asset =
            Some(ImportedImage::new(image, "L"));
        assert!(s.trim(&TrimOptions::default()));
        assert_eq!(s.document.as_ref().unwrap().width, 20);
    }

    #[test]
    fn palette_mask_painting_is_black_and_white() {
        let mut s = session();
        assert_eq!(s.palette_color(false), PaletteColor::BLACK);
        s.is_mask_selected = true;
        s.mask_paint_white = true;
        assert_eq!(s.palette_color(false), PaletteColor::WHITE);
        assert_eq!(s.palette_color(true), PaletteColor::BLACK);
        s.swap_palette_colors();
        assert_eq!(s.palette_color(false), PaletteColor::BLACK);
    }

    #[test]
    fn wand_settings_default_matches_photoshop() {
        let w = WandSettings::default();
        assert_eq!(w.tolerance, 32);
        assert!(w.contiguous);
        assert_eq!(w.sample_radius, 0);
    }
}
